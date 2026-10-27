//! The graph, and the walk through your files that builds it.
//!
//! The walk goes one import level at a time: first the entrypoints, then the files
//! they import, then the files those import, and so on. Each file is opened once,
//! even if many files import it, which is also what stops import cycles from looping.
//!
//! For each file it opens:
//! 1. Skip it if it is not code (`.css`, `.json`...): it is shown but never opened.
//! 2. Find its imports (`parse`), keeping only the kinds in `Config::edge_kinds`.
//! 3. Find the file each import points to (`resolve`). Not found: noted, not fatal.
//! 4. Decide whether to show it: packages and runtime modules follow `Externals`;
//!    files follow `include`/`exclude`, except files already in the graph, so
//!    entrypoints are always kept and arrows into them too.
//! 5. Add the arrow, and queue the file for the next level if it is new.
//!
//! Steps 1 to 3 run in parallel for all files of a level. Steps 4 and 5
//! run on one thread, in a fixed order, so the result never depends on timing.
//!
//! The walk stops after `Config::max_depth` levels. At the end, files are sorted by
//! name, so every run on the same code gives the same graph.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use oxc_allocator::Allocator;
use oxc_span::SourceType;
use rayon::prelude::*;

use crate::{
    config::{Config, EdgeKinds, Externals},
    error::{Error, Result},
    parse::parse,
    resolve::{Resolver, Target},
};

/// The files reachable from the entrypoints, and which file imports which.
///
/// Built by [`build_graph`](crate::build_graph). Files are [`Module`]s, imports are
/// [`Edge`]s between them. Problems found along the way are kept too:
/// [`unresolved`](Self::unresolved) imports and [`diagnostics`](Self::diagnostics).
///
/// ```no_run
/// let graph = oxchart::build_graph(&oxchart::Config::new(["src/index.ts"]))?;
/// for edge in graph.edges() {
///     println!("{:?} imports {:?}", graph.module(edge.from), graph.module(edge.to));
/// }
/// # Ok::<(), oxchart::Error>(())
/// ```
#[derive(Debug)]
pub struct ModuleGraph {
    root: PathBuf,
    modules: Vec<Module>,
    edges: Vec<Edge>,
    unresolved: Vec<Unresolved>,
    diagnostics: Vec<Diagnostic>,
}

impl ModuleGraph {
    /// The project folder, as an absolute path. Useful to show file paths shorter.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Every file, package and runtime module in the graph, sorted.
    ///
    /// A module's position in this list is its [`ModuleId`].
    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    /// The module with this id.
    pub fn module(&self, id: ModuleId) -> &Module {
        &self.modules[id.index()]
    }

    /// Every import arrow, sorted by `from`, then `to`.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Imports that point to nothing: a missing file, or a package that is not installed.
    pub fn unresolved(&self) -> &[Unresolved] {
        &self.unresolved
    }

    /// Problems inside files: syntax errors, files that cannot be read, and imports
    /// whose path is computed at run time, like `require(name)`.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// A module's number: its position in [`ModuleGraph::modules`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModuleId(u32);

impl ModuleId {
    fn new(index: usize) -> Self {
        Self(u32::try_from(index).expect("fewer than 2^32 modules"))
    }

    /// The position in [`ModuleGraph::modules`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One node of the graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Module {
    /// A file, as an absolute path. Your own code, a file inside a package with
    /// [`Externals::Follow`], or a file that is not code, like a `.css`.
    File(PathBuf),
    /// A whole npm package, like `react` or `@scope/lib`. Only with [`Externals::Leaf`].
    Package(String),
    /// A module the runtime provides, like `node:fs` or `bun:sqlite`. Only with
    /// [`Externals::Leaf`] or [`Externals::Follow`].
    Runtime(String),
}

/// An import arrow: `from` imports `to`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Edge {
    /// The file that imports.
    pub from: ModuleId,
    /// What it imports.
    pub to: ModuleId,
    /// Every way `from` imports `to`. One arrow carries them all.
    pub kinds: EdgeKinds,
}

/// An import that points to nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unresolved {
    /// The file with the import.
    pub from: ModuleId,
    /// The import as written, like `./missing` or `left-pad`.
    pub specifier: String,
    /// Why it was not found, for people to read.
    pub reason: String,
}

/// A problem inside one file. The file stays in the graph; it may miss some imports.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    /// The file with the problem.
    pub module: ModuleId,
    /// What is wrong, for people to read.
    pub message: String,
    /// Where it is, counting from 1, when known.
    pub line: Option<usize>,
}

pub(crate) fn build(config: &Config) -> Result<ModuleGraph> {
    config.validate()?;
    let root = config.root_dir()?;
    let filter = config.path_filter()?;
    let resolver = Resolver::new(config);
    let mut graph = Builder::default();

    let mut level = Vec::new();
    for entry in &config.entrypoints {
        let path = root
            .join(entry)
            .canonicalize()
            .map_err(|source| Error::Entrypoint {
                path: entry.clone(),
                source,
            })?;
        if let (id, true) = graph.add(Module::File(path.clone())) {
            level.push((id, path));
        }
    }

    let mut depth = 0;
    while !level.is_empty() && config.max_depth.is_none_or(|max| depth < max) {
        // Files of one level are opened in parallel. `map_init` reuses an allocator for
        // many files instead of making one per file.
        let opened: Vec<Opened> = level
            .par_iter()
            .map_init(Allocator::default, |allocator, (from, path)| {
                open(*from, path, config, &resolver, allocator)
            })
            .collect();

        // Merged on one thread, in the same order every run, so ids are stable.
        let mut next = Vec::new();
        for file in opened {
            graph.diagnostics.extend(file.diagnostics);
            for (specifier, kinds, target) in file.imports {
                let target = match target {
                    Ok(target) => target,
                    Err(reason) => {
                        graph.unresolved.push(Unresolved {
                            from: file.from,
                            specifier,
                            reason,
                        });
                        continue;
                    }
                };
                let (module, open) = match target {
                    Target::Local(path) => (Module::File(path.clone()), Some(path)),
                    Target::Package { path, name } => match config.externals {
                        Externals::Hide => continue,
                        Externals::Leaf => (Module::Package(name), None),
                        Externals::Follow => (Module::File(path.clone()), Some(path)),
                    },
                    Target::Runtime(name) => match config.externals {
                        Externals::Hide => continue,
                        Externals::Leaf | Externals::Follow => (Module::Runtime(name), None),
                    },
                };
                // Files already in the graph skip the filter: that keeps arrows into
                // excluded entrypoints.
                if let (Module::File(path), None) = (&module, graph.ids.get(&module))
                    && !filter.is_match(path.strip_prefix(&root).unwrap_or(path))
                {
                    continue;
                }
                let (to, new) = graph.add(module);
                *graph
                    .edges
                    .entry((file.from, to))
                    .or_insert(EdgeKinds::NONE) |= kinds;
                if new && let Some(path) = open {
                    next.push((to, path));
                }
            }
        }
        level = next;
        depth += 1;
    }
    Ok(graph.finish(root))
}

/// What one file imports, ready to merge into the graph.
struct Opened {
    from: ModuleId,
    /// Each import as written, the kinds it was found with, and where it leads (or why
    /// it leads nowhere).
    imports: Vec<(String, EdgeKinds, Result<Target, String>)>,
    diagnostics: Vec<Diagnostic>,
}

/// Reads one file, finds its imports and where each one leads. Runs on any thread.
fn open(
    from: ModuleId,
    path: &Path,
    config: &Config,
    resolver: &Resolver,
    allocator: &mut Allocator,
) -> Opened {
    let mut opened = Opened {
        from,
        imports: Vec::new(),
        diagnostics: Vec::new(),
    };
    // Not JavaScript or TypeScript (`.css`, `.json`...): shown, never opened.
    let Ok(source_type) = SourceType::from_path(path) else {
        return opened;
    };
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            opened.diagnostics.push(Diagnostic {
                module: from,
                message: format!("cannot read file: {err}"),
                line: None,
            });
            return opened;
        }
    };
    allocator.reset();
    let parsed = parse(allocator, source_type, &source, config.edge_kinds);
    opened
        .diagnostics
        .extend(parsed.diagnostics.into_iter().map(|d| {
            Diagnostic {
                module: from,
                line: d
                    .labels
                    .first()
                    .map(|label| line_of(&source, label.offset())),
                message: d.message.to_string(),
            }
        }));
    for import in parsed.imports {
        for (kinds, require) in split_require(import.kinds) {
            let target = resolver
                .resolve(path, &import.specifier, require)
                .map_err(|err| err.to_string());
            opened
                .imports
                .push((import.specifier.clone(), kinds, target));
        }
    }
    opened
}

/// The graph while it is being built. Ids follow the order files were found in.
#[derive(Default)]
struct Builder {
    modules: Vec<Module>,
    ids: HashMap<Module, ModuleId>,
    edges: HashMap<(ModuleId, ModuleId), EdgeKinds>,
    unresolved: Vec<Unresolved>,
    diagnostics: Vec<Diagnostic>,
}

impl Builder {
    /// The id of `module`, and `true` if it was not in the graph yet.
    fn add(&mut self, module: Module) -> (ModuleId, bool) {
        if let Some(&id) = self.ids.get(&module) {
            return (id, false);
        }
        let id = ModuleId::new(self.modules.len());
        self.modules.push(module.clone());
        self.ids.insert(module, id);
        (id, true)
    }

    /// Sorts modules by name and arrows by (from, to), so every run gives the same graph.
    fn finish(self, root: PathBuf) -> ModuleGraph {
        let mut sorted: Vec<(Module, usize)> = self.modules.into_iter().zip(0..).collect();
        sorted.sort();
        let mut new_id = vec![ModuleId(0); sorted.len()];
        for (new, (_, old)) in sorted.iter().enumerate() {
            new_id[*old] = ModuleId::new(new);
        }
        let renumber = |id: ModuleId| new_id[id.index()];

        let mut edges: Vec<Edge> = self
            .edges
            .into_iter()
            .map(|((from, to), kinds)| Edge {
                from: renumber(from),
                to: renumber(to),
                kinds,
            })
            .collect();
        edges.sort_by_key(|e| (e.from, e.to));
        // Stable sorts: problems in one file stay in the order they were found.
        let mut unresolved = self.unresolved;
        unresolved
            .iter_mut()
            .for_each(|u| u.from = renumber(u.from));
        unresolved.sort_by_key(|u| u.from);
        let mut diagnostics = self.diagnostics;
        diagnostics
            .iter_mut()
            .for_each(|d| d.module = renumber(d.module));
        diagnostics.sort_by_key(|d| d.module);

        ModuleGraph {
            root,
            modules: sorted.into_iter().map(|(module, _)| module).collect(),
            edges,
            unresolved,
            diagnostics,
        }
    }
}

/// Splits `kinds` by the resolver they need: `import` kinds, then `require`.
///
/// A package can ship one file for `import` and another for `require()`, so a file
/// that imports a package both ways may point to two different files.
fn split_require(kinds: EdgeKinds) -> impl Iterator<Item = (EdgeKinds, bool)> {
    let import = [
        EdgeKinds::STATIC,
        EdgeKinds::DYNAMIC,
        EdgeKinds::TYPE_ONLY,
        EdgeKinds::RE_EXPORT,
    ]
    .into_iter()
    .filter(|&kind| kinds.contains(kind))
    .fold(EdgeKinds::NONE, |all, kind| all | kind);
    let require = if kinds.contains(EdgeKinds::REQUIRE) {
        EdgeKinds::REQUIRE
    } else {
        EdgeKinds::NONE
    };
    [(import, false), (require, true)]
        .into_iter()
        .filter(|(kinds, _)| !kinds.is_empty())
}

/// The 1-based line of a byte offset.
fn line_of(source: &str, offset: u32) -> usize {
    let end = (offset as usize).min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolve::tests::project;

    const S: EdgeKinds = EdgeKinds::STATIC;
    const X: EdgeKinds = EdgeKinds::RE_EXPORT;
    const R: EdgeKinds = EdgeKinds::REQUIRE;

    fn config(root: &Path) -> Config {
        let mut config = Config::new(["src/index.ts"]);
        config.root = Some(root.into());
        config
    }

    fn name(g: &ModuleGraph, id: ModuleId) -> String {
        match g.module(id) {
            Module::File(path) => path.strip_prefix(g.root()).unwrap().display().to_string(),
            Module::Package(name) => format!("pkg:{name}"),
            Module::Runtime(name) => name.clone(),
        }
    }

    fn names(g: &ModuleGraph) -> Vec<String> {
        (0..g.modules().len())
            .map(|i| name(g, ModuleId::new(i)))
            .collect()
    }

    fn edges(g: &ModuleGraph) -> Vec<(String, String, EdgeKinds)> {
        g.edges()
            .iter()
            .map(|e| (name(g, e.from), name(g, e.to), e.kinds))
            .collect()
    }

    fn edge(from: &str, to: &str, kinds: EdgeKinds) -> (String, String, EdgeKinds) {
        (from.into(), to.into(), kinds)
    }

    /// `index` imports `a` twice and requires `b`; `a` and `b` import each other.
    fn cycle(name: &str) -> PathBuf {
        project(
            name,
            &[
                (
                    "src/index.ts",
                    "import './a'; export * from './a'; require('./b');",
                ),
                ("src/a.ts", "import './b';"),
                ("src/b.ts", "import './a';"),
            ],
        )
    }

    #[test]
    fn walks_imports_merges_kinds_and_survives_cycles() {
        let root = cycle("graph-cycle");
        let g = build(&config(&root)).unwrap();
        assert_eq!(names(&g), ["src/a.ts", "src/b.ts", "src/index.ts"]);
        assert_eq!(
            edges(&g),
            [
                edge("src/a.ts", "src/b.ts", S),
                edge("src/b.ts", "src/a.ts", S),
                edge("src/index.ts", "src/a.ts", S | X),
                edge("src/index.ts", "src/b.ts", R),
            ]
        );
    }

    #[test]
    fn max_depth_stops_the_walk() {
        let root = cycle("graph-depth");
        let mut c = config(&root);
        c.max_depth = Some(0);
        let g = build(&c).unwrap();
        assert_eq!(names(&g), ["src/index.ts"]);
        assert!(g.edges().is_empty());

        c.max_depth = Some(1);
        let g = build(&c).unwrap();
        assert_eq!(names(&g), ["src/a.ts", "src/b.ts", "src/index.ts"]);
        // `a` and `b` are shown but not opened, so their imports are missing.
        assert_eq!(g.edges().len(), 2);
    }

    #[test]
    fn externals() {
        let root = project(
            "graph-externals",
            &[
                (
                    "src/index.ts",
                    "import 'react'; import 'react/jsx-runtime'; import 'fs'; import 'bun:sqlite';",
                ),
                (
                    "node_modules/react/package.json",
                    r#"{ "main": "./index.js" }"#,
                ),
                ("node_modules/react/index.js", "require('./jsx-runtime');"),
                ("node_modules/react/jsx-runtime.js", ""),
            ],
        );
        let mut c = config(&root);
        let g = build(&c).unwrap();
        assert_eq!(names(&g), ["src/index.ts"]);

        c.externals = Externals::Leaf;
        let g = build(&c).unwrap();
        assert_eq!(
            names(&g),
            ["src/index.ts", "pkg:react", "bun:sqlite", "node:fs"]
        );
        assert_eq!(g.edges().len(), 3);

        c.externals = Externals::Follow;
        let g = build(&c).unwrap();
        assert_eq!(
            names(&g),
            [
                "node_modules/react/index.js",
                "node_modules/react/jsx-runtime.js",
                "src/index.ts",
                "bun:sqlite",
                "node:fs",
            ]
        );
        // Followed packages are opened like your own files.
        assert!(edges(&g).contains(&edge(
            "node_modules/react/index.js",
            "node_modules/react/jsx-runtime.js",
            R
        )));
    }

    #[test]
    fn scoped_packages_keep_their_scope() {
        let root = project(
            "graph-scoped",
            &[
                ("src/index.ts", "import '@scope/lib/sub';"),
                ("node_modules/@scope/lib/sub.js", ""),
            ],
        );
        let mut c = config(&root);
        c.externals = Externals::Leaf;
        assert_eq!(
            names(&build(&c).unwrap()),
            ["src/index.ts", "pkg:@scope/lib"]
        );
    }

    #[test]
    fn include_and_exclude() {
        let root = project(
            "graph-filter",
            &[
                ("src/index.ts", "import './a'; import './a.test';"),
                ("src/a.ts", "import '../index';"),
                ("src/a.test.ts", "import './hidden';"),
                ("src/hidden.ts", ""),
                ("index.ts", ""),
            ],
        );
        let mut c = config(&root);
        c.include = vec!["src/**".into()];
        c.exclude = vec!["**/*.test.ts".into(), "src/index.ts".into()];
        let g = build(&c).unwrap();
        // The entrypoint is kept even though it is excluded; `index.ts` is outside `src/**`.
        assert_eq!(names(&g), ["src/a.ts", "src/index.ts"]);
    }

    #[test]
    fn imports_into_an_excluded_entrypoint_keep_their_arrow() {
        let root = project(
            "graph-excluded-entry",
            &[
                ("src/index.ts", "import './a';"),
                ("src/a.ts", "import './index';"),
            ],
        );
        let mut c = config(&root);
        c.exclude = vec!["src/index.ts".into()];
        let g = build(&c).unwrap();
        assert_eq!(
            edges(&g),
            [
                edge("src/a.ts", "src/index.ts", S),
                edge("src/index.ts", "src/a.ts", S)
            ]
        );
    }

    #[test]
    fn files_that_are_not_code_are_shown_not_opened() {
        let root = project(
            "graph-assets",
            &[
                (
                    "src/index.ts",
                    "import './style.css'; import './data.json';",
                ),
                ("src/style.css", "body { color: red }"),
                ("src/data.json", "{}"),
            ],
        );
        let g = build(&config(&root)).unwrap();
        assert_eq!(
            names(&g),
            ["src/data.json", "src/index.ts", "src/style.css"]
        );
        assert!(g.diagnostics().is_empty());
    }

    #[test]
    fn problems_are_reported_not_fatal() {
        let root = project(
            "graph-problems",
            &[
                (
                    "src/index.ts",
                    "import './missing';\nimport './broken';\nrequire(name);",
                ),
                ("src/broken.ts", "\n\nconst = ;"),
            ],
        );
        let g = build(&config(&root)).unwrap();
        assert_eq!(names(&g), ["src/broken.ts", "src/index.ts"]);

        let [missing] = g.unresolved() else {
            panic!("{:?}", g.unresolved())
        };
        assert_eq!(name(&g, missing.from), "src/index.ts");
        assert_eq!(missing.specifier, "./missing");

        let found: Vec<_> = g
            .diagnostics()
            .iter()
            .map(|d| (name(&g, d.module), d.line))
            .collect();
        assert_eq!(
            found,
            [
                ("src/broken.ts".into(), Some(3)),
                ("src/index.ts".into(), Some(3))
            ]
        );
    }

    #[test]
    fn import_and_require_of_one_package_can_reach_two_files() {
        let exports = r#"{ "exports": { "import": "./esm.mjs", "require": "./cjs.cjs" } }"#;
        let root = project(
            "graph-dual",
            &[
                ("src/index.ts", "import 'dual'; require('dual');"),
                ("node_modules/dual/package.json", exports),
                ("node_modules/dual/esm.mjs", ""),
                ("node_modules/dual/cjs.cjs", ""),
            ],
        );
        let mut c = config(&root);
        c.externals = Externals::Follow;
        let g = build(&c).unwrap();
        assert_eq!(
            edges(&g),
            [
                edge("src/index.ts", "node_modules/dual/cjs.cjs", R),
                edge("src/index.ts", "node_modules/dual/esm.mjs", S),
            ]
        );
    }

    #[test]
    fn missing_entrypoint_is_an_error() {
        let root = project("graph-no-entry", &[("src/a.ts", "")]);
        assert!(matches!(
            build(&config(&root)),
            Err(Error::Entrypoint { .. })
        ));
    }
}
