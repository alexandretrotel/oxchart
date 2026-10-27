//! Command-line flags.

use std::path::{PathBuf, absolute};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use oxchart::{
    Config, EdgeKinds, Externals,
    render::{Mermaid, Renderer, Text},
};
use serde::Deserialize;

const EXAMPLES: &str = "\
Examples:
  oxchart src/index.ts                    Graph of your own files
  oxchart src/main.ts --externals leaf    Also show packages, one node each
  oxchart src/index.ts --max-depth 2      Stop two imports away from the start
  oxchart src/index.ts -o graph.txt       Write the graph to a file
  oxchart src/index.ts --format mermaid   Mermaid chart, for GitHub or mermaid.live

Settings can also live in oxchart.toml, in this folder or a parent one.
Flags win over the file.";

/// See which files of a JavaScript or TypeScript project import which.
///
/// Prints one line per import to the terminal. Warnings and a summary go to stderr,
/// so piping the output stays clean.
/// How to write the graph. Set with `--format` or `format` in oxchart.toml.
#[derive(Debug, Clone, Copy, Default, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// One line per import
    #[default]
    Text,
    /// A Mermaid flowchart
    Mermaid,
}

impl Format {
    pub fn renderer(self) -> Box<dyn Renderer> {
        match self {
            Self::Text => Box::new(Text::default()),
            Self::Mermaid => Box::new(Mermaid::default()),
        }
    }
}

#[derive(Debug, Parser)]
#[command(version, after_help = EXAMPLES)]
pub struct Args {
    /// Files to start from, like src/index.ts
    pub entrypoints: Vec<PathBuf>,

    /// Read settings from this file instead of looking for oxchart.toml
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Do not read oxchart.toml
    #[arg(long, conflicts_with = "config")]
    pub no_config: bool,

    /// Project folder: globs start from here [default: folder of oxchart.toml, or .]
    #[arg(long, value_name = "DIR")]
    pub root: Option<PathBuf>,

    /// Extensions to try, in order, when an import has none [default: .ts,.tsx,...]
    #[arg(long = "ext", value_name = "EXT", value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,

    /// tsconfig to use for every file [default: nearest tsconfig.json of each file]
    #[arg(long, value_name = "FILE")]
    pub tsconfig: Option<PathBuf>,

    /// Package build to pick, like browser; repeat or use commas [default: node]
    #[arg(long = "condition", value_name = "NAME", value_delimiter = ',')]
    pub conditions: Option<Vec<String>>,

    /// Keep only files matching this glob, like 'src/**'; repeat for more
    #[arg(long, value_name = "GLOB")]
    pub include: Option<Vec<String>>,

    /// Leave out files matching this glob, like '**/*.test.ts'; repeat for more
    #[arg(long, value_name = "GLOB")]
    pub exclude: Option<Vec<String>>,

    /// How many imports away from an entrypoint to go [default: no limit]
    #[arg(long, value_name = "N")]
    pub max_depth: Option<usize>,

    /// Packages and runtime modules: hide, leaf or follow [default: hide]
    #[arg(long, value_name = "MODE")]
    pub externals: Option<Externals>,

    /// Kinds of import to follow: static, dynamic, type, re-export, require, all
    /// [default: static,re-export,require]
    #[arg(
        long,
        value_name = "KINDS",
        value_delimiter = ',',
        verbatim_doc_comment
    )]
    pub edges: Option<Vec<EdgeKinds>>,

    /// How to write the graph [default: text]
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<Format>,

    /// Write the graph to this file instead of the terminal
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,

    /// Print only the graph: no warnings, no summary
    #[arg(short, long)]
    pub quiet: bool,
}

impl Args {
    /// Writes the flags that were given into `config`, over the values from oxchart.toml.
    ///
    /// Paths are made absolute here, so they start from where the command runs, even
    /// when the root comes from an oxchart.toml in a parent folder.
    pub fn apply(&self, config: &mut Config) -> Result<()> {
        if !self.entrypoints.is_empty() {
            config.entrypoints = self
                .entrypoints
                .iter()
                .map(absolute)
                .collect::<Result<_, _>>()?;
        }
        if let Some(root) = &self.root {
            config.root = Some(absolute(root)?);
        }
        if let Some(extensions) = &self.extensions {
            config.extensions.clone_from(extensions);
        }
        if let Some(tsconfig) = &self.tsconfig {
            config.tsconfig = Some(absolute(tsconfig)?);
        }
        if let Some(conditions) = &self.conditions {
            config.conditions.clone_from(conditions);
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
        if let Some(externals) = self.externals {
            config.externals = externals;
        }
        if let Some(edges) = &self.edges {
            config.edge_kinds = edges.iter().fold(EdgeKinds::NONE, |all, &kind| all | kind);
        }
        Ok(())
    }
}
