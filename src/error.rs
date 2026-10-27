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
    // TODO(resolve): an entrypoint that does not resolve is fatal (`Entrypoint { path, source }`).
    // Unresolved imports inside modules are NOT errors: they go to `ModuleGraph::unresolved`.
    // Same for parse errors: collect as diagnostics, keep walking.
}

/// Shorthand for `Result<T, oxchart::Error>`.
pub type Result<T, E = Error> = std::result::Result<T, E>;
