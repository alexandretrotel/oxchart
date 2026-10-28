//! `oxchart.toml`: settings saved in the project, so you do not retype flags.
//!
//! ```toml
//! entrypoints = ["src/index.ts"]
//! exclude = ["**/*.test.ts"]
//! externals = "leaf"
//! edges = ["static", "re-export", "require"]
//! max-depth = 4
//! format = "mermaid"
//! ```

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use oxchart::{Config, EdgeKinds};
use serde::Deserialize;

use crate::args::{Format, manual_tsconfig};

/// Every setting is optional: a missing one keeps the default.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct File {
    entrypoints: Option<Vec<PathBuf>>,
    root: Option<PathBuf>,
    extensions: Option<Vec<String>>,
    tsconfig: Option<PathBuf>,
    condition_names: Option<Vec<String>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    max_depth: Option<usize>,
    externals: Option<String>,
    edges: Option<Vec<String>>,
    pub format: Option<Format>,
}

/// Looks for `oxchart.toml` in `dir`, then in each parent folder, like `tsconfig.json`.
pub fn find(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|dir| dir.join("oxchart.toml"))
        .find(|path| path.is_file())
}

pub fn load(path: &Path) -> Result<File> {
    let text =
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))
}

impl File {
    /// Writes the settings into `config`. `path` is the file, as an absolute path.
    ///
    /// The root is the file's folder unless the file says otherwise, and every other
    /// path in the file starts from the root.
    pub fn apply(&self, path: &Path, config: &mut Config) -> Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        let invalid = |key: &str| format!("invalid `{key}` in {}", path.display());

        config.root = Some(match &self.root {
            Some(root) => dir.join(root),
            None => dir.to_path_buf(),
        });
        if let Some(entrypoints) = &self.entrypoints {
            config.entrypoints.clone_from(entrypoints);
        }
        if let Some(extensions) = &self.extensions {
            config.resolve.extensions.clone_from(extensions);
        }
        if let (Some(tsconfig), Some(root)) = (&self.tsconfig, &config.root) {
            config.resolve.tsconfig = manual_tsconfig(root.join(tsconfig));
        }
        if let Some(condition_names) = &self.condition_names {
            config.resolve.condition_names.clone_from(condition_names);
        }
        if let Some(include) = &self.include {
            config.include.clone_from(include);
        }
        if let Some(exclude) = &self.exclude {
            config.exclude.clone_from(exclude);
        }
        if self.max_depth.is_some() {
            config.max_depth = self.max_depth;
        }
        if let Some(externals) = &self.externals {
            config.externals = externals.parse().with_context(|| invalid("externals"))?;
        }
        if let Some(edges) = &self.edges {
            config.edge_kinds = edges
                .iter()
                .map(|name| name.parse::<EdgeKinds>())
                .try_fold(EdgeKinds::NONE, |all, kind| kind.map(|kind| all | kind))
                .with_context(|| invalid("edges"))?;
        }
        Ok(())
    }
}
