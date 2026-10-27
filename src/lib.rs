//! Build module graphs from entrypoints.
//!
//! ```no_run
//! let config = oxchart::Config::new(["src/index.ts"]);
//! let graph = oxchart::build_graph(&config)?;
//! # Ok::<(), oxchart::Error>(())
//! ```
//!
//! Parsing and resolution come from [oxc](https://oxc.rs) by the VoidZero team.
//! The graph holds no rendering data; renderers (Mermaid via `merman` first) read it.

mod config;
mod error;
mod graph;
mod parse;
mod resolve;
// TODO(render): `render` module, `Renderer` trait and `DisplayOptions` (deferred).

pub use config::{Config, EdgeKinds, Externals};
pub use error::{Error, Result};
pub use graph::ModuleGraph;

/// Walks every module reachable from `config.entrypoints`.
pub fn build_graph(config: &Config) -> Result<ModuleGraph> {
    graph::build(config)
}
