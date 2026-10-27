//! Turns an import path into a real file on disk, using `oxc_resolver`.
//!
//! `import x from './a'` only says `./a`. This module finds which file that is:
//! `./a.ts`, `./a/index.ts`, a file inside an npm package, or something the runtime
//! provides, like Node's `fs` or Bun's `bun:sqlite`.
//!
//! How it looks:
//! - From the folder of the file that imports, not from the project root.
//! - Trying each extension in `Config::extensions` when the import leaves it out.
//! - Reading TypeScript's habit of writing the compiled name: `./a.js` finds `./a.ts`.
//! - Using the aliases from the tsconfig: the nearest `tsconfig.json` above each file,
//!   or the one in `Config::tsconfig` for every file.
//! - For a package, picking the build that matches `Config::conditions`. Packages
//!   without `exports` are read from their `module` field, then `main`.
//! - Treating imports with a scheme (`node:`, `bun:`, `cloudflare:`, Deno's `jsr:`,
//!   `npm:` and `https:`), plus bare `bun`, as provided by the runtime. They are not
//!   files on disk, so they are never looked up.
//! - Following symlinks to the real file. Paths then start like `Config::root_dir`,
//!   and monorepo packages (symlinks to your own folders) count as your own code.
//!
//! What it does not do: decide what an error means. A missing import is reported by
//! the graph; only a missing entrypoint stops the build.

use std::path::{Path, PathBuf};

use oxc_resolver::{
    ResolveError, ResolveOptions, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};

use crate::config::Config;

/// Where an import leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A file of your own code.
    Local(PathBuf),
    /// A file inside an npm package, and the package's name, like `react`.
    Package { path: PathBuf, name: String },
    /// Provided by the runtime, not a file: `node:fs` (also written `fs`), `bun`,
    /// `bun:sqlite`, `cloudflare:workers`, Deno's `jsr:@std/path`...
    Runtime(String),
}

/// Finds the files that imports point to. Build it once, then share it between threads.
pub(crate) struct Resolver {
    /// For `import` and `import()`: adds the `import` condition.
    import: oxc_resolver::Resolver,
    /// For `require()`: adds the `require` condition. Shares `import`'s cache, so no
    /// folder or `package.json` is read twice.
    require: oxc_resolver::Resolver,
}

impl Resolver {
    /// Sets up the lookup from the user's settings.
    pub(crate) fn new(config: &Config) -> Self {
        let conditions = |extra: &str| {
            let mut names = config.conditions.clone();
            names.push(extra.into());
            names
        };
        // TypeScript code imports `./a.js` to mean `./a.ts`: try the TypeScript file first.
        let alias = |from: &str, to: &[&str]| (from.into(), to.iter().map(|&e| e.into()).collect());
        let options = ResolveOptions {
            extensions: config.extensions.clone(),
            extension_alias: vec![
                alias(".js", &[".ts", ".tsx", ".js"]),
                alias(".jsx", &[".tsx", ".jsx"]),
                alias(".mjs", &[".mts", ".mjs"]),
                alias(".cjs", &[".cts", ".cjs"]),
            ],
            tsconfig: Some(match &config.tsconfig {
                Some(path) => TsconfigDiscovery::Manual(TsconfigOptions {
                    // A relative result is read from the current directory, like `root`.
                    config_file: config.root.as_deref().unwrap_or(Path::new(".")).join(path),
                    references: TsconfigReferences::Auto,
                }),
                // Each file uses its nearest `tsconfig.json`, like TypeScript does.
                None => TsconfigDiscovery::Auto,
            }),
            condition_names: conditions("import"),
            main_fields: vec!["module".into(), "main".into()],
            // Report `fs`, `node:fs`... as built-ins instead of looking for them on disk.
            builtin_modules: true,
            ..ResolveOptions::default()
        };
        let import = oxc_resolver::Resolver::new(options.clone());
        let require = import.clone_with_options(ResolveOptions {
            condition_names: conditions("require"),
            ..options
        });
        Self { import, require }
    }

    /// Finds the file `specifier` points to, as imported from the file `importer`.
    ///
    /// `require` picks the build for `require()` instead of `import`: a package can ship
    /// a different file for each. A file that imports a package both ways needs two calls.
    ///
    /// An error means the import was not found; the caller decides whether that matters.
    pub(crate) fn resolve(
        &self,
        importer: &Path,
        specifier: &str,
        require: bool,
    ) -> Result<Target, ResolveError> {
        if specifier == "bun" || has_scheme(specifier) {
            return Ok(Target::Runtime(specifier.into()));
        }
        let resolver = if require { &self.require } else { &self.import };
        match resolver.resolve_file(importer, specifier) {
            Ok(resolution) => {
                let path = resolution.path();
                // The path is the real one (symlinks followed), so a monorepo package
                // lives in your own folders and is not a package.
                if !path.components().any(|c| c.as_os_str() == "node_modules") {
                    return Ok(Target::Local(resolution.into_path_buf()));
                }
                let name = resolution
                    .package_json()
                    .and_then(|package| package.name())
                    .map_or_else(|| package_name(path), String::from);
                Ok(Target::Package {
                    path: resolution.into_path_buf(),
                    name,
                })
            }
            // A bare Node built-in like `fs`, reported as `node:fs`.
            Err(ResolveError::Builtin { resolved, .. }) => Ok(Target::Runtime(resolved)),
            Err(err) => Err(err),
        }
    }
}

/// Guesses a package's name from its folder: `react` or `@scope/lib`.
///
/// Only used when the nearest `package.json` has no name, like the small ones some
/// packages put in `dist/cjs` to set `"type"`. Uses the last `node_modules` in the path,
/// so pnpm's `node_modules/.pnpm/react@18/node_modules/react/index.js` gives `react`.
fn package_name(path: &Path) -> String {
    let parts: Vec<_> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    let start = parts
        .iter()
        .rposition(|c| c == "node_modules")
        .map_or(0, |i| i + 1);
    match &parts[start..] {
        [scope, name, ..] if scope.starts_with('@') => format!("{scope}/{name}"),
        [name, ..] => name.to_string(),
        [] => String::new(),
    }
}

/// `true` if `specifier` starts with a URL scheme like `bun:` or `https:`.
///
/// A scheme is a letter, then letters, digits, `+`, `-` or `.`, then `:`. It needs at
/// least two characters, so a Windows drive like `C:` is not one.
fn has_scheme(specifier: &str) -> bool {
    specifier.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() > 1
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use super::*;

    /// A small project in a fresh temp folder: `(path, content)` pairs.
    pub(crate) fn project(name: &str, files: &[(&str, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("oxchart-test-{name}"));
        let _ = fs::remove_dir_all(&root);
        for (path, content) in files {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        root.canonicalize().unwrap()
    }

    fn resolver() -> Resolver {
        Resolver::new(&Config::new(["src/index.ts"]))
    }

    fn local(root: &Path, path: &str) -> Result<Target, ResolveError> {
        Ok(Target::Local(root.join(path)))
    }

    fn package(root: &Path, path: &str, name: &str) -> Result<Target, ResolveError> {
        Ok(Target::Package {
            path: root.join(path),
            name: name.into(),
        })
    }

    #[test]
    fn relative_imports() {
        let root = project(
            "relative",
            &[
                ("src/index.ts", ""),
                ("src/a.ts", ""),
                ("src/b.tsx", ""),
                ("src/c.mts", ""),
                ("src/utils/index.ts", ""),
            ],
        );
        let r = resolver();
        let from = root.join("src/index.ts");
        assert_eq!(r.resolve(&from, "./a", false), local(&root, "src/a.ts"));
        assert_eq!(
            r.resolve(&from, "./utils", false),
            local(&root, "src/utils/index.ts")
        );
        // TypeScript ESM code writes the extension the file will have once compiled.
        assert_eq!(r.resolve(&from, "./a.js", false), local(&root, "src/a.ts"));
        assert_eq!(r.resolve(&from, "./b.js", false), local(&root, "src/b.tsx"));
        assert_eq!(
            r.resolve(&from, "./c.mjs", false),
            local(&root, "src/c.mts")
        );
        // Relative to the importing file, not the root.
        let nested = root.join("src/utils/index.ts");
        assert_eq!(r.resolve(&nested, "../a", false), local(&root, "src/a.ts"));
    }

    #[test]
    fn tsconfig_paths_without_base_url() {
        let root = project(
            "tsconfig",
            &[
                ("src/index.ts", ""),
                ("src/a.ts", ""),
                (
                    "tsconfig.json",
                    r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#,
                ),
            ],
        );
        let mut config = Config::new(["src/index.ts"]);
        config.root = Some(root.clone());
        config.tsconfig = Some("tsconfig.json".into());
        let r = Resolver::new(&config);
        let from = root.join("src/index.ts");
        assert_eq!(r.resolve(&from, "@/a", false), local(&root, "src/a.ts"));
    }

    #[test]
    fn tsconfig_is_found_per_folder() {
        let paths = r#"{ "compilerOptions": { "paths": { "@/*": ["./src/*"] } } }"#;
        let root = project(
            "tsconfig-auto",
            &[
                ("tsconfig.json", paths),
                ("src/index.ts", ""),
                ("src/a.ts", ""),
                ("packages/ui/tsconfig.json", paths),
                ("packages/ui/src/index.ts", ""),
                ("packages/ui/src/a.ts", ""),
            ],
        );
        let r = resolver();
        let app = root.join("src/index.ts");
        let ui = root.join("packages/ui/src/index.ts");
        assert_eq!(r.resolve(&app, "@/a", false), local(&root, "src/a.ts"));
        assert_eq!(
            r.resolve(&ui, "@/a", false),
            local(&root, "packages/ui/src/a.ts")
        );
    }

    #[test]
    fn package_builds() {
        let exports = r#"{ "exports": { "node": { "import": "./node.mjs", "require": "./node.cjs" },
                                        "browser": "./browser.js", "default": "./default.js" } }"#;
        let root = project(
            "packages",
            &[
                ("src/index.ts", ""),
                ("node_modules/pkg/package.json", exports),
                ("node_modules/pkg/node.mjs", ""),
                ("node_modules/pkg/node.cjs", ""),
                ("node_modules/pkg/browser.js", ""),
                ("node_modules/pkg/default.js", ""),
                (
                    "node_modules/web/package.json",
                    r#"{ "exports": { "browser": "./b.js", "default": "./d.js" } }"#,
                ),
                ("node_modules/web/b.js", ""),
                ("node_modules/web/d.js", ""),
                (
                    "node_modules/old/package.json",
                    r#"{ "module": "./esm.js", "main": "./cjs.js" }"#,
                ),
                ("node_modules/old/esm.js", ""),
                ("node_modules/old/cjs.js", ""),
            ],
        );
        let r = resolver();
        let from = root.join("src/index.ts");
        assert_eq!(
            r.resolve(&from, "pkg", false),
            package(&root, "node_modules/pkg/node.mjs", "pkg")
        );
        assert_eq!(
            r.resolve(&from, "pkg", true),
            package(&root, "node_modules/pkg/node.cjs", "pkg")
        );
        // `default` matches without being listed in `conditions`.
        assert_eq!(
            r.resolve(&from, "web", false),
            package(&root, "node_modules/web/d.js", "web")
        );
        // No `exports`: `module` wins over `main`.
        assert_eq!(
            r.resolve(&from, "old", false),
            package(&root, "node_modules/old/esm.js", "old")
        );
    }

    #[test]
    fn runtime_modules() {
        let root = project("runtime", &[("src/index.ts", "")]);
        let r = resolver();
        let from = root.join("src/index.ts");
        let runtime = |name: &str| Ok(Target::Runtime(name.into()));
        assert_eq!(r.resolve(&from, "fs", false), runtime("node:fs"));
        assert_eq!(r.resolve(&from, "node:fs", true), runtime("node:fs"));
        assert_eq!(r.resolve(&from, "bun", false), runtime("bun"));
        assert_eq!(r.resolve(&from, "bun:sqlite", false), runtime("bun:sqlite"));
        assert_eq!(
            r.resolve(&from, "cloudflare:workers", false),
            runtime("cloudflare:workers")
        );
        assert_eq!(
            r.resolve(&from, "jsr:@std/path", false),
            runtime("jsr:@std/path")
        );
        assert_eq!(
            r.resolve(&from, "npm:react@18", false),
            runtime("npm:react@18")
        );
        assert_eq!(
            r.resolve(&from, "https://deno.land/x/a.ts", false),
            runtime("https://deno.land/x/a.ts")
        );
    }

    #[test]
    fn colons_that_are_not_schemes() {
        let root = project("not-schemes", &[("src/index.ts", "")]);
        let r = resolver();
        let from = root.join("src/index.ts");
        // A Windows drive letter, and names that only contain a colon later on.
        assert!(r.resolve(&from, "C:/nope.ts", false).is_err());
        assert!(r.resolve(&from, "./a:b", false).is_err());
        assert!(r.resolve(&from, "@scope/a:b", false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn package_names_come_from_package_json() {
        // Yarn's pnpm linker stores packages under `node_modules/.store`: the folder
        // name says nothing about the package, its `package.json` does.
        let root = project(
            "package-names",
            &[
                ("src/index.ts", ""),
                (
                    "node_modules/.store/react-npm-18/package/package.json",
                    r#"{ "name": "react" }"#,
                ),
                ("node_modules/.store/react-npm-18/package/index.js", ""),
                (
                    "node_modules/dual/package.json",
                    r#"{ "name": "dual", "main": "./dist/cjs/index.js" }"#,
                ),
                (
                    "node_modules/dual/dist/cjs/package.json",
                    r#"{ "type": "commonjs" }"#,
                ),
                ("node_modules/dual/dist/cjs/index.js", ""),
            ],
        );
        std::os::unix::fs::symlink(
            root.join("node_modules/.store/react-npm-18/package"),
            root.join("node_modules/react"),
        )
        .unwrap();
        let r = resolver();
        let from = root.join("src/index.ts");
        assert_eq!(
            r.resolve(&from, "react", false),
            package(
                &root,
                "node_modules/.store/react-npm-18/package/index.js",
                "react"
            )
        );
        // The nearest `package.json` has no name: fall back to the folder name.
        assert_eq!(
            r.resolve(&from, "dual", false),
            package(&root, "node_modules/dual/dist/cjs/index.js", "dual")
        );
    }

    #[cfg(unix)]
    #[test]
    fn monorepo_packages_are_local() {
        let root = project(
            "workspace",
            &[
                ("src/index.ts", ""),
                ("packages/lib/package.json", r#"{ "main": "./index.ts" }"#),
                ("packages/lib/index.ts", ""),
            ],
        );
        fs::create_dir_all(root.join("node_modules")).unwrap();
        std::os::unix::fs::symlink(root.join("packages/lib"), root.join("node_modules/lib"))
            .unwrap();
        let r = resolver();
        let from = root.join("src/index.ts");
        assert_eq!(
            r.resolve(&from, "lib", false),
            local(&root, "packages/lib/index.ts")
        );
    }

    #[test]
    fn missing_imports_are_errors() {
        let root = project("missing", &[("src/index.ts", "")]);
        let r = resolver();
        let from = root.join("src/index.ts");
        assert!(r.resolve(&from, "./nope", false).is_err());
        assert!(r.resolve(&from, "nope", false).is_err());
    }
}
