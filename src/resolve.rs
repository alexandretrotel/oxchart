//! Turns an import path into a real file on disk, using `oxc_resolver`.
//!
//! `import x from './a'` only says `./a`. This module finds which file that is:
//! `./a.ts`, `./a/index.ts`, a file inside an npm package, or something the runtime
//! provides, like Node's `fs` or Bun's `bun:sqlite`.
//!
//! How it looks:
//! - From the folder of the file that imports, not from the project root.
//! - With `Config::resolve`, passed as-is to `oxc_resolver`: extensions, tsconfig
//!   aliases, package conditions... Only the `import` or `require` condition is added.
//! - Treating imports with a known scheme ([`RUNTIME_SCHEMES`]: `node:`, `bun:`,
//!   `cloudflare:`, Deno's `jsr:`...), plus bare `bun`, as provided by the runtime,
//!   unless a tsconfig alias matches them first. Any other `a:b` that is not found is
//!   reported like a missing file, so a typo is not hidden.
//! - Following symlinks to the real file. Paths then start like `Config::root_dir`,
//!   and monorepo packages (symlinks to your own folders) count as your own code.
//!
//! What it does not do: decide what an error means. A missing import is reported by
//! the graph; only a missing entrypoint stops the build.

use std::path::{Path, PathBuf};

use crate::{config::Config, graph::ResolveError};

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
    /// Sets up the lookup from [`Config::resolve`], adding the `import` or `require`
    /// condition.
    pub(crate) fn new(config: &Config) -> Self {
        let with_condition = |extra: &str| {
            let mut options = config.resolve.clone();
            options.condition_names.push(extra.into());
            options
        };
        let import = oxc_resolver::Resolver::new(with_condition("import"));
        let require = import.clone_with_options(with_condition("require"));
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
        if specifier == "bun" {
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
            // Not on disk and not a tsconfig alias: `bun:sqlite`, `jsr:@std/path`...
            Err(ResolveError::NotFound(_)) if is_runtime_scheme(specifier) => {
                Ok(Target::Runtime(specifier.into()))
            }
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

/// Schemes whose imports the runtime or the bundler provides, not a file. `file:` is
/// not one: those are paths, found like any other.
const RUNTIME_SCHEMES: &[&str] = &[
    "node",
    "bun",
    "cloudflare",
    "jsr",
    "npm",
    "http",
    "https",
    "data",
    // Astro's `astro:content`, and Vite plugins' `virtual:pwa-register`.
    "astro",
    "virtual",
];

/// `true` if `specifier` starts with a scheme in [`RUNTIME_SCHEMES`], like `bun:`.
fn is_runtime_scheme(specifier: &str) -> bool {
    specifier
        .split_once(':')
        .is_some_and(|(scheme, _)| RUNTIME_SCHEMES.contains(&scheme))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::fs;

    use crate::config::{TsconfigDiscovery, TsconfigOptions, TsconfigReferences};

    use super::*;

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
        assert_eq!(r.resolve(&from, "./a.js", false), local(&root, "src/a.ts"));
        assert_eq!(r.resolve(&from, "./b.js", false), local(&root, "src/b.tsx"));
        assert_eq!(
            r.resolve(&from, "./c.mjs", false),
            local(&root, "src/c.mts")
        );
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
        config.resolve.tsconfig = Some(TsconfigDiscovery::Manual(TsconfigOptions {
            config_file: root.join("tsconfig.json"),
            references: TsconfigReferences::Auto,
        }));
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
        assert_eq!(
            r.resolve(&from, "web", false),
            package(&root, "node_modules/web/d.js", "web")
        );
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
        assert_eq!(
            r.resolve(&from, "astro:content", false),
            runtime("astro:content")
        );
        assert_eq!(
            r.resolve(&from, "virtual:pwa-register", false),
            runtime("virtual:pwa-register")
        );
    }

    #[test]
    fn tsconfig_alias_wins_over_scheme() {
        let root = project(
            "scheme-alias",
            &[
                ("src/index.ts", ""),
                ("src/utils/a.ts", ""),
                (
                    "tsconfig.json",
                    r#"{ "compilerOptions": { "paths": { "app:utils/*": ["./src/utils/*"] } } }"#,
                ),
            ],
        );
        let from = root.join("src/index.ts");
        assert_eq!(
            resolver().resolve(&from, "app:utils/a", false),
            local(&root, "src/utils/a.ts")
        );
    }

    #[test]
    fn colons_that_are_not_schemes() {
        let root = project("not-schemes", &[("src/index.ts", "")]);
        let r = resolver();
        let from = root.join("src/index.ts");
        assert!(r.resolve(&from, "C:/nope.ts", false).is_err());
        assert!(r.resolve(&from, "./a:b", false).is_err());
        assert!(r.resolve(&from, "@scope/a:b", false).is_err());
        // Unknown schemes are reported, so a typo or a broken alias is not hidden.
        assert!(r.resolve(&from, "foo:bar", false).is_err());
        assert!(r.resolve(&from, "src:utils", false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn package_names_come_from_package_json() {
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
