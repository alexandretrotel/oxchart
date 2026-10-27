//! See which files of a JavaScript or TypeScript project import which.
//!
//! Give oxchart the file your app starts from (its *entrypoint*). It reads that file,
//! follows every import, then the imports of those files, and so on. The result is a
//! [`ModuleGraph`]: each file is a node, each import is an arrow between two nodes.
//!
//! ```no_run
//! use oxchart::{Config, EdgeKinds};
//!
//! let mut config = Config::new(["src/index.ts"]);
//! config.max_depth = Some(3); // stop three imports away from `src/index.ts`
//! config.edge_kinds |= EdgeKinds::DYNAMIC; // also follow `import('./page')`
//!
//! let graph = oxchart::build_graph(&config)?;
//! # Ok::<(), oxchart::Error>(())
//! ```
//!
//! The defaults in [`Config::new`] work for most projects; change a field only when
//! you need to.
//!
//! Reading files and finding imported files is done by [oxc](https://oxc.rs), made by
//! the VoidZero team.

mod config;
mod error;
mod graph;
mod parse;
mod resolve;
// TODO(render): `render` module, `Renderer` trait and `DisplayOptions` (deferred).

pub use config::{Config, EdgeKinds, Externals};
pub use error::{Error, Result};
pub use graph::{Diagnostic, Edge, Module, ModuleGraph, ModuleId, Unresolved};

/// Builds the graph of every file reachable from the entrypoints.
///
/// An import that cannot be found does not stop the build. Neither does a file with a
/// syntax error. Both are reported in the returned graph.
///
/// # Errors
///
/// Fails if the settings are invalid (see [`Config::validate`]), the root directory
/// cannot be opened, or an entrypoint does not exist.
pub fn build_graph(config: &Config) -> Result<ModuleGraph> {
    graph::build(config)
}
