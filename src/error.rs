//! What can go wrong while building a graph.

use std::{io, path::PathBuf};

/// Why [`build_graph`](crate::build_graph) could not build a graph.
///
/// Problems inside your files, like a missing import or a syntax error, are not errors:
/// the graph still builds and reports them.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// [`Config::entrypoints`](crate::Config::entrypoints) is empty, so there is nothing to start from.
    #[error("no entrypoints given")]
    NoEntrypoints,

    /// A pattern in `include` or `exclude` is not a valid glob, like `src/[a.ts`.
    #[error("invalid glob pattern `{pattern}`")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },

    /// The root directory does not exist or cannot be read.
    #[error("cannot open root directory {}", path.display())]
    Root {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A setting has a value oxchart does not know, like `skip` for `externals`.
    #[error("unknown value `{value}`, expected one of: {expected}")]
    UnknownValue {
        value: String,
        expected: &'static str,
    },

    /// [`render`](crate::render) could not write the graph, for example because the
    /// output file or pipe was closed, or merman could not draw it.
    #[cfg(feature = "render")]
    #[error("cannot write the graph")]
    Render(#[source] Box<dyn std::error::Error + Send + Sync>),

    /// An entrypoint does not exist. Entrypoints are file paths: `src/index` does not
    /// find `src/index.ts`.
    #[error("entrypoint not found: {}", path.display())]
    Entrypoint {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Shorthand for `Result<T, oxchart::Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;
