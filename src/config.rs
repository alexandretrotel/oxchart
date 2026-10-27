//! Settings for building a graph.

use std::{
    fmt,
    ops::{BitOr, BitOrAssign},
    path::{Path, PathBuf},
    str::FromStr,
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::{Error, Result};

/// Settings for [`build_graph`](crate::build_graph).
///
/// Start from [`Config::new`] and change only the fields you need. The defaults give a
/// graph of your own code as it runs:
///
/// - the imports that load code when the app starts, but not `import()` or `import type`
/// - no npm packages or runtime modules (`node:fs`, `bun:sqlite`...)
/// - no depth limit
///
/// ```
/// let mut config = oxchart::Config::new(["src/main.ts"]);
/// config.exclude.push("**/*.test.ts".into());
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// The files to start from, usually your app's main file.
    ///
    /// Relative paths start from [`root`](Self::root).
    pub entrypoints: Vec<PathBuf>,
    /// The project folder. Relative paths in this config start from here.
    ///
    /// `None` uses the current directory.
    pub root: Option<PathBuf>,
    /// File extensions to try, in order, when an import leaves one out.
    ///
    /// With the default list, `import './a'` finds `./a.ts` before `./a.js`.
    pub extensions: Vec<String>,
    /// The tsconfig whose aliases to use, like `import x from '@/utils'`.
    ///
    /// `None` finds it for you: each file uses the nearest `tsconfig.json` above it,
    /// like TypeScript does. That works for monorepos where each package has its own.
    /// Set a path only to use a file with another name, like `tsconfig.build.json`,
    /// for every file. Both `compilerOptions.paths` and the older `baseUrl` work.
    pub tsconfig: Option<PathBuf>,
    /// Which build of a package to pick when it ships several.
    ///
    /// Some packages publish different files for Node, for browsers, and so on, keyed
    /// by name in their `package.json` `exports`. The default, `["node"]`, picks the
    /// Node build. For a browser app, add `"browser"`.
    ///
    /// `"import"` or `"require"` is added for you, depending on how each file imports
    /// the package. `"default"` always matches, so you never need to list it.
    pub conditions: Vec<String>,
    /// Keep only files matching these patterns, like `src/**`.
    ///
    /// Patterns start from [`root`](Self::root). `*` stays in one folder, `**` crosses
    /// folders. Empty keeps every file. Entrypoints are always kept.
    pub include: Vec<String>,
    /// Leave out files matching these patterns, like `**/*.test.ts`.
    ///
    /// A left-out file is not shown and its own imports are not followed. This wins over
    /// [`include`](Self::include).
    pub exclude: Vec<String>,
    /// How many imports away from an entrypoint to go.
    ///
    /// `Some(1)` shows the entrypoints and the files they import directly. `Some(0)`
    /// shows the entrypoints only. `None` has no limit.
    pub max_depth: Option<usize>,
    /// What to show for npm packages and runtime modules. See [`Externals`].
    pub externals: Externals,
    /// Which kinds of import to show and follow. See [`EdgeKinds`].
    pub edge_kinds: EdgeKinds,
}

impl Config {
    /// Settings with sensible defaults, starting from these files.
    pub fn new<I, P>(entrypoints: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        Self {
            entrypoints: entrypoints.into_iter().map(Into::into).collect(),
            root: None,
            extensions: [
                ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json",
            ]
            .map(String::from)
            .into(),
            tsconfig: None,
            conditions: vec!["node".into()],
            include: Vec::new(),
            exclude: Vec::new(),
            max_depth: None,
            externals: Externals::default(),
            edge_kinds: EdgeKinds::STATIC | EdgeKinds::RE_EXPORT | EdgeKinds::REQUIRE,
        }
    }

    /// Checks the settings without reading any file.
    ///
    /// [`build_graph`](crate::build_graph) calls this for you.
    ///
    /// # Errors
    ///
    /// Fails if there are no entrypoints, or a pattern in `include` or `exclude` is not
    /// a valid glob.
    pub fn validate(&self) -> Result<()> {
        if self.entrypoints.is_empty() {
            return Err(Error::NoEntrypoints);
        }
        self.path_filter().map(drop)
    }

    /// The root directory, absolute and with symlinks resolved, so it prefixes resolver output.
    pub(crate) fn root_dir(&self) -> Result<PathBuf> {
        let path = self.root.as_deref().unwrap_or(Path::new("."));
        path.canonicalize().map_err(|source| Error::Root {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Compiles `include` and `exclude` once, for matching every visited file.
    pub(crate) fn path_filter(&self) -> Result<PathFilter> {
        Ok(PathFilter {
            include: (!self.include.is_empty())
                .then(|| glob_set(&self.include))
                .transpose()?,
            exclude: glob_set(&self.exclude)?,
        })
    }
}

/// `*` stops at `/`, `**` crosses directories, like `.gitignore`.
fn glob_set(patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .map_err(|source| Error::Glob {
                pattern: pattern.clone(),
                source,
            })?;
        set.add(glob);
    }
    set.build().map_err(|source| Error::Glob {
        pattern: patterns.join(", "),
        source,
    })
}

/// What to show for imports of npm packages (`react`) and of modules the runtime
/// provides (`node:fs`, `bun:sqlite`, `cloudflare:workers`...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Externals {
    /// Leave them out. The graph shows only your own files. This is the default.
    #[default]
    Hide,
    /// Show each package as one node, without looking inside it.
    ///
    /// `react` and `react/jsx-runtime` both point to a single `react` node.
    Leaf,
    /// Look inside packages too, following their imports like your own files.
    ///
    /// Graphs get very big, so pair this with [`Config::max_depth`].
    Follow,
}

/// Kinds of import, combined with `|`.
///
/// Used in two places: [`Config::edge_kinds`] says which kinds to follow, and each
/// arrow in the graph says how one file imports the other. A file that imports
/// another in two ways gets one arrow with both kinds.
///
/// ```
/// use oxchart::EdgeKinds;
///
/// let kinds = EdgeKinds::STATIC | EdgeKinds::DYNAMIC;
/// assert!(kinds.contains(EdgeKinds::DYNAMIC));
/// assert!(!kinds.contains(EdgeKinds::REQUIRE));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeKinds(u8);

impl EdgeKinds {
    /// No kind.
    pub const NONE: Self = Self(0);
    /// A plain import: `import x from './a'` or `import './a'`.
    pub const STATIC: Self = Self(1);
    /// An import that loads later, when the code runs: `import('./a')`.
    ///
    /// Off by default. Often used to load a page or a heavy feature on demand.
    pub const DYNAMIC: Self = Self(1 << 1);
    /// An import of types only: `import type { A } from './a'`.
    ///
    /// Off by default: TypeScript removes these imports when it compiles, so they never
    /// load. `import { type A, type B }` counts too, but `import { type A, b }` is a plain
    /// import because `b` is real code.
    pub const TYPE_ONLY: Self = Self(1 << 2);
    /// Passing another file's exports along: `export * from './a'`.
    ///
    /// Typical in `index.ts` files that gather a folder's exports in one place.
    pub const RE_EXPORT: Self = Self(1 << 3);
    /// A CommonJS import: `require('./a')`.
    pub const REQUIRE: Self = Self(1 << 4);
    /// Every kind.
    pub const ALL: Self = Self(0b1_1111);

    /// `true` if every kind in `other` is also in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` if `self` and `other` have at least one kind in common.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// `true` if there is no kind at all.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for EdgeKinds {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for EdgeKinds {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Reads `hide`, `leaf` or `follow`, as written in `oxchart.toml` or on the command line.
impl FromStr for Externals {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "hide" => Ok(Self::Hide),
            "leaf" => Ok(Self::Leaf),
            "follow" => Ok(Self::Follow),
            _ => Err(Error::UnknownValue {
                value: s.into(),
                expected: "hide, leaf, follow",
            }),
        }
    }
}

/// The name of each kind, as written in `oxchart.toml`, on the command line and in output.
const KIND_NAMES: [(EdgeKinds, &str); 5] = [
    (EdgeKinds::STATIC, "static"),
    (EdgeKinds::DYNAMIC, "dynamic"),
    (EdgeKinds::TYPE_ONLY, "type"),
    (EdgeKinds::RE_EXPORT, "re-export"),
    (EdgeKinds::REQUIRE, "require"),
];

/// Reads one kind by name, like `static` or `re-export`, or `all` for every kind.
impl FromStr for EdgeKinds {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s == "all" {
            return Ok(Self::ALL);
        }
        KIND_NAMES
            .iter()
            .find(|(_, name)| *name == s)
            .map(|(kind, _)| *kind)
            .ok_or_else(|| Error::UnknownValue {
                value: s.into(),
                expected: "static, dynamic, type, re-export, require, all",
            })
    }
}

/// Writes the names of the kinds, like `static, re-export`, or `none`.
impl fmt::Display for EdgeKinds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        let mut names = KIND_NAMES.iter().filter(|(kind, _)| self.contains(*kind));
        if let Some((_, first)) = names.next() {
            f.write_str(first)?;
        }
        names.try_for_each(|(_, name)| write!(f, ", {name}"))
    }
}

/// Compiled `include`/`exclude` globs.
#[derive(Debug)]
pub(crate) struct PathFilter {
    include: Option<GlobSet>,
    exclude: GlobSet,
}

impl PathFilter {
    /// `rel` must be relative to the root, as globs are.
    pub(crate) fn is_match(&self, rel: &Path) -> bool {
        self.include.as_ref().is_none_or(|set| set.is_match(rel)) && !self.exclude.is_match(rel)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn config() -> Config {
        Config::new(["src/index.ts"])
    }

    #[test]
    fn defaults_keep_runtime_edges_only() {
        let c = config();
        assert!(
            c.edge_kinds
                .contains(EdgeKinds::STATIC | EdgeKinds::RE_EXPORT)
        );
        assert!(c.edge_kinds.contains(EdgeKinds::REQUIRE));
        assert!(
            !c.edge_kinds
                .intersects(EdgeKinds::DYNAMIC | EdgeKinds::TYPE_ONLY)
        );
        assert_eq!(c.externals, Externals::Hide);
        assert_eq!(c.max_depth, None);
    }

    #[test]
    fn defaults_cover_ts_jsx_esm_and_cjs() {
        let c = config();
        for ext in [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"] {
            assert!(c.extensions.iter().any(|e| e == ext), "missing {ext}");
        }
    }

    #[test]
    fn edge_kinds_set_ops() {
        let mut k = EdgeKinds::NONE;
        assert!(k.is_empty());
        k |= EdgeKinds::STATIC;
        k |= EdgeKinds::STATIC;
        assert_eq!(k, EdgeKinds::STATIC);
        assert!(EdgeKinds::ALL.contains(k | EdgeKinds::REQUIRE));
        assert!(!k.intersects(EdgeKinds::DYNAMIC));
    }

    #[test]
    fn validate_accepts_defaults() {
        config().validate().unwrap();
    }

    #[test]
    fn validate_rejects_no_entrypoints() {
        let c = Config::new(Vec::<PathBuf>::new());
        assert!(matches!(c.validate(), Err(Error::NoEntrypoints)));
    }

    #[test]
    fn validate_rejects_bad_glob() {
        let mut c = config();
        c.exclude.push("src/[".into());
        assert!(matches!(c.validate(), Err(Error::Glob { pattern, .. }) if pattern == "src/["));
    }

    #[test]
    fn empty_include_matches_everything() {
        let f = config().path_filter().unwrap();
        assert!(f.is_match(Path::new("src/a.ts")));
        assert!(f.is_match(Path::new("deep/er/b.js")));
    }

    #[test]
    fn include_and_exclude() {
        let mut c = config();
        c.include = vec!["src/**".into()];
        c.exclude = vec!["**/*.test.ts".into()];
        let f = c.path_filter().unwrap();
        assert!(f.is_match(Path::new("src/a/b.ts")));
        assert!(!f.is_match(Path::new("lib/a.ts")));
        assert!(!f.is_match(Path::new("src/a.test.ts")));
    }

    #[test]
    fn single_star_stays_in_one_directory() {
        let mut c = config();
        c.include = vec!["src/*.ts".into()];
        let f = c.path_filter().unwrap();
        assert!(f.is_match(Path::new("src/a.ts")));
        assert!(!f.is_match(Path::new("src/nested/a.ts")));
    }

    #[test]
    fn root_dir_defaults_to_cwd() {
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(config().root_dir().unwrap(), cwd);
    }

    #[test]
    fn root_dir_is_canonical() {
        let tmp = std::env::temp_dir().join("oxchart-root-dir-test");
        fs::create_dir_all(&tmp).unwrap();
        let mut c = config();
        c.root = Some(tmp.join("."));
        assert_eq!(c.root_dir().unwrap(), tmp.canonicalize().unwrap());
    }

    #[test]
    fn root_dir_must_exist() {
        let mut c = config();
        c.root = Some("does/not/exist".into());
        assert!(matches!(c.root_dir(), Err(Error::Root { .. })));
    }

    #[test]
    fn externals_from_text() {
        assert_eq!("hide".parse::<Externals>().unwrap(), Externals::Hide);
        assert_eq!("leaf".parse::<Externals>().unwrap(), Externals::Leaf);
        assert_eq!("follow".parse::<Externals>().unwrap(), Externals::Follow);
        let err = "skip".parse::<Externals>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "unknown value `skip`, expected one of: hide, leaf, follow"
        );
    }

    #[test]
    fn edge_kinds_from_text_and_back() {
        let parse = |s: &str| s.parse::<EdgeKinds>().unwrap();
        assert_eq!(parse("static"), EdgeKinds::STATIC);
        assert_eq!(parse("dynamic"), EdgeKinds::DYNAMIC);
        assert_eq!(parse("type"), EdgeKinds::TYPE_ONLY);
        assert_eq!(parse("re-export"), EdgeKinds::RE_EXPORT);
        assert_eq!(parse("require"), EdgeKinds::REQUIRE);
        assert_eq!(parse("all"), EdgeKinds::ALL);
        assert!("statik".parse::<EdgeKinds>().is_err());

        assert_eq!(
            (EdgeKinds::STATIC | EdgeKinds::RE_EXPORT).to_string(),
            "static, re-export"
        );
        assert_eq!(EdgeKinds::NONE.to_string(), "none");
    }
}
