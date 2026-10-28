//! Command-line flags.

use std::{
    io::Write,
    path::{Path, PathBuf, absolute},
};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use oxchart::{
    Config, EdgeKinds, Externals, ModuleGraph, TsconfigDiscovery, TsconfigOptions,
    TsconfigReferences,
    render::{Mermaid, Renderer, Text},
};
use serde::Deserialize;

const EXAMPLES: &str = "\
Examples:
  oxchart src/index.ts                    Open a picture of your own files
  oxchart src/main.ts --externals leaf    Also show packages, one node each
  oxchart src/index.ts --max-depth 2      Stop two imports away from the start
  oxchart src/index.ts -o graph.svg       Save the picture instead of opening it
  oxchart src/index.ts -o graph.png       Also .jpg and .pdf
  oxchart src/index.ts -o graph.mmd       Mermaid chart, for GitHub or mermaid.live
  oxchart src/index.ts --format ascii     Draw the chart in the terminal
  oxchart src/index.ts | grep react       One line per import, when piped

Settings can also live in oxchart.toml, in this folder or a parent one.
Flags win over the file.";

/// How to write the graph. Set with `--format` or `format` in oxchart.toml.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// One line per import
    Text,
    /// A Mermaid flowchart
    Mermaid,
    /// A picture of the Mermaid flowchart
    Svg,
    /// The Mermaid flowchart drawn with text, for the terminal
    Ascii,
    /// A PNG image of the Mermaid flowchart
    Png,
    /// A JPEG image of the Mermaid flowchart
    Jpeg,
    /// A PDF of the Mermaid flowchart
    Pdf,
}

impl Format {
    /// The format a file name asks for: `.svg`, `.mmd` or `.mermaid`, else text.
    pub fn of(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("svg") => Self::Svg,
            Some("png") => Self::Png,
            Some("jpg" | "jpeg") => Self::Jpeg,
            Some("pdf") => Self::Pdf,
            Some("mmd" | "mermaid") => Self::Mermaid,
            _ => Self::Text,
        }
    }

    /// The file extension of formats that are opened in a viewer, not printed.
    pub fn picture(self) -> Option<&'static str> {
        match self {
            Self::Svg => Some("svg"),
            Self::Png => Some("png"),
            Self::Jpeg => Some("jpg"),
            Self::Pdf => Some("pdf"),
            Self::Text | Self::Mermaid | Self::Ascii => None,
        }
    }

    pub fn write(self, graph: &ModuleGraph, out: &mut dyn Write) -> Result<()> {
        let mermaid = Mermaid::default();
        match self {
            Self::Text => Text::default().render(graph, out)?,
            Self::Mermaid => mermaid.render(graph, out)?,
            Self::Svg => out.write_all(mermaid.render_svg(graph)?.as_bytes())?,
            Self::Ascii => out.write_all(mermaid.render_ascii(graph)?.as_bytes())?,
            Self::Png => out.write_all(&mermaid.render_png(graph)?)?,
            Self::Jpeg => out.write_all(&mermaid.render_jpeg(graph)?)?,
            Self::Pdf => out.write_all(&mermaid.render_pdf(graph)?)?,
        }
        Ok(())
    }
}

/// See which files of a JavaScript or TypeScript project import which.
///
/// In a terminal, draws the graph as a picture and opens it. When the output is piped
/// or saved with -o, writes it instead. Warnings and a summary go to stderr.
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
    #[arg(long, value_name = "EXT", value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,

    /// tsconfig to use for every file [default: nearest tsconfig.json of each file]
    #[arg(long, value_name = "FILE")]
    pub tsconfig: Option<PathBuf>,

    /// Package build to pick, like browser; repeat or use commas [default: node]
    #[arg(long, value_name = "NAME", value_delimiter = ',')]
    pub condition_names: Option<Vec<String>>,

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

    /// How to write the graph [default: svg, opened, in a terminal; text when piped;
    /// from the extension with -o]
    #[arg(long, value_name = "FORMAT", verbatim_doc_comment)]
    pub format: Option<Format>,

    /// Save the graph to this file; .svg, .png, .jpg, .pdf, .mmd and .txt pick the format
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
            config.resolve.extensions.clone_from(extensions);
        }
        if let Some(tsconfig) = &self.tsconfig {
            config.resolve.tsconfig = manual_tsconfig(absolute(tsconfig)?);
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
        if let Some(externals) = self.externals {
            config.externals = externals;
        }
        if let Some(edges) = &self.edges {
            config.edge_kinds = edges.iter().fold(EdgeKinds::NONE, |all, &kind| all | kind);
        }
        Ok(())
    }
}

/// A tsconfig used for every file, instead of the nearest one.
pub fn manual_tsconfig(config_file: PathBuf) -> Option<TsconfigDiscovery> {
    Some(TsconfigDiscovery::Manual(TsconfigOptions {
        config_file,
        references: TsconfigReferences::Auto,
    }))
}
