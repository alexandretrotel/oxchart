//! Graph-building settings.

use std::{
    ops::{BitOr, BitOrAssign},
    path::{Path, PathBuf},
};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::error::{Error, Result};

/// Settings for [`build_graph`](crate::build_graph).
///
/// [`Config::new`] gives defaults tuned for a clean graph of your own runtime code:
/// static imports, re-exports and `require` only, `node_modules` hidden, no depth limit.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// Files the walk starts from. Relative paths are joined to the root.
    pub entrypoints: Vec<PathBuf>,
    /// Base for relative entrypoints and globs. `None` means the current directory.
    pub root: Option<PathBuf>,
    /// Extensions tried, in order, for extensionless specifiers (`./a` -> `./a.ts`).
    pub extensions: Vec<String>,
    /// `tsconfig.json` whose `paths` (and legacy `baseUrl`) define import aliases.
    pub tsconfig: Option<PathBuf>,
    /// `package.json` `exports`/`imports` conditions. `import` or `require` is added per edge;
    /// `default` always matches.
    pub conditions: Vec<String>,
    /// Globs relative to the root. Empty means every file.
    pub include: Vec<String>,
    /// Globs relative to the root. A file matching both lists is excluded.
    pub exclude: Vec<String>,
    /// Max import hops from an entrypoint. `None` is unlimited, `Some(0)` keeps entrypoints only.
    pub max_depth: Option<usize>,
    /// What to do with packages and Node builtins.
    pub externals: Externals,
    /// Which import kinds become edges.
    pub edge_kinds: EdgeKinds,
}

impl Config {
    /// Default settings for the given entrypoints.
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

    /// Checks the settings without touching the file system.
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

/// What happens to imports that leave your source: `react`, `node:fs`, `fs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Externals {
    /// Drop them. The graph shows only your code.
    #[default]
    Hide,
    /// Keep one node per package or builtin, never parsed.
    Leaf,
    /// Walk into `node_modules` like local code. Graphs get big fast.
    Follow,
}

/// A set of import kinds. One edge carries every kind that links its two modules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EdgeKinds(u8);

impl EdgeKinds {
    pub const NONE: Self = Self(0);
    /// `import x from './a'`, `import './a'`.
    pub const STATIC: Self = Self(1);
    /// `import('./a')`.
    pub const DYNAMIC: Self = Self(1 << 1);
    /// `import type`, `export type ... from`, or an import whose specifiers are all `type`.
    pub const TYPE_ONLY: Self = Self(1 << 2);
    /// `export ... from './a'`, `export * from './a'`.
    pub const RE_EXPORT: Self = Self(1 << 3);
    /// `require('./a')`.
    pub const REQUIRE: Self = Self(1 << 4);
    pub const ALL: Self = Self(0b1_1111);

    /// True if every kind in `other` is in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// True if `self` and `other` share a kind.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

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
}
