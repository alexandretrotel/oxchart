//! Library errors. The CLI wraps these in `anyhow` for context.

use std::{io, path::PathBuf};

/// Everything that can go wrong while building a graph.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("no entrypoints given")]
    NoEntrypoints,

    #[error("invalid glob pattern `{pattern}`")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },

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

pub type Result<T, E = Error> = std::result::Result<T, E>;
