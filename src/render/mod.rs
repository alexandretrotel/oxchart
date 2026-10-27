//! Turns a graph into text you can read or paste somewhere.
//!
//! Pick a [`Renderer`] and give it a graph:
//!
//! ```no_run
//! use oxchart::render::{Mermaid, Renderer};
//!
//! let graph = oxchart::build_graph(&oxchart::Config::new(["src/index.ts"]))?;
//! let chart = Mermaid::default().render_to_string(&graph)?;
//! # Ok::<(), oxchart::Error>(())
//! ```

mod mermaid;
mod text;

use std::io::Write;

pub use mermaid::Mermaid;
pub use text::Text;

use crate::{
    error::{Error, Result},
    graph::ModuleGraph,
};

/// A way to write a graph: [`Text`] or [`Mermaid`].
///
/// Each renderer keeps its own options in its fields. It can be stored as
/// `Box<dyn Renderer>`, to pick one at run time.
pub trait Renderer {
    /// Writes `graph` to `out`, like a file or the terminal.
    ///
    /// The graph is written piece by piece, so even a big one is never held in memory
    /// as a whole.
    ///
    /// # Errors
    ///
    /// [`Error::Render`] if `out` cannot be written to.
    fn render(&self, graph: &ModuleGraph, out: &mut dyn Write) -> Result<()>;

    /// Writes `graph` into a `String`.
    ///
    /// # Errors
    ///
    /// [`Error::Render`] if the renderer fails.
    fn render_to_string(&self, graph: &ModuleGraph) -> Result<String> {
        let mut out = Vec::new();
        self.render(graph, &mut out)?;
        String::from_utf8(out).map_err(|err| Error::Render(err.into()))
    }
}

/// Wraps a write error, for renderers.
fn render_error(err: std::io::Error) -> Error {
    Error::Render(err.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, Externals},
        resolve::tests::project,
    };

    /// `index` imports `a` and `react`, `a` re-exports `b`, and `lone` imports nothing.
    fn graph(name: &str) -> ModuleGraph {
        let root = project(
            name,
            &[
                ("src/index.ts", "import './a'; import 'react'; import 'fs';"),
                (
                    "src/a.ts",
                    "export * from './b'; import type { T } from './b';",
                ),
                ("src/b.ts", ""),
                ("src/lone.ts", ""),
                ("node_modules/react/index.js", ""),
            ],
        );
        let mut config = Config::new(["src/index.ts", "src/lone.ts"]);
        config.root = Some(root);
        config.externals = Externals::Leaf;
        config.edge_kinds = crate::EdgeKinds::ALL;
        crate::build_graph(&config).unwrap()
    }

    #[test]
    fn text() {
        let out = Text::default()
            .render_to_string(&graph("render-text"))
            .unwrap();
        assert_eq!(
            out,
            "src/a.ts -> src/b.ts (type, re-export)\n\
             src/index.ts -> src/a.ts (static)\n\
             src/index.ts -> react (static)\n\
             src/index.ts -> node:fs (static)\n\
             src/lone.ts\n"
        );
    }

    #[test]
    fn mermaid() {
        let out = Mermaid::default()
            .render_to_string(&graph("render-mermaid"))
            .unwrap();
        assert_eq!(
            out,
            "flowchart LR\n\
             \x20 m0[\"src/a.ts\"]\n\
             \x20 m1[\"src/b.ts\"]\n\
             \x20 m2[\"src/index.ts\"]\n\
             \x20 m3[\"src/lone.ts\"]\n\
             \x20 m4([\"react\"])\n\
             \x20 m5[[\"node:fs\"]]\n\
             \x20 m0 -->|type, re-export| m1\n\
             \x20 m2 -->|static| m0\n\
             \x20 m2 -->|static| m4\n\
             \x20 m2 -->|static| m5\n"
        );
    }

    #[test]
    fn mermaid_escapes_quotes() {
        let root = project("render-quotes", &[("a\"b.ts", "")]);
        let mut config = Config::new(["a\"b.ts"]);
        config.root = Some(root);
        let graph = crate::build_graph(&config).unwrap();
        let out = Mermaid::default().render_to_string(&graph).unwrap();
        assert!(out.contains("m0[\"a#quot;b.ts\"]"), "{out}");
    }

    #[cfg(feature = "svg")]
    #[test]
    fn mermaid_svg() {
        let svg = Mermaid::default().render_svg(&graph("render-svg")).unwrap();
        assert!(svg.starts_with("<svg"), "{}", &svg[..svg.len().min(200)]);
        assert!(svg.contains("src/index.ts"));
        assert!(svg.contains("react"));
        assert!(!svg.contains("<foreignObject"));
        assert!(svg.contains("<text"));
    }

    #[test]
    fn write_errors_become_render_errors() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let graph = graph("render-closed");
        let err = Text::default().render(&graph, &mut Closed).unwrap_err();
        assert!(matches!(err, crate::Error::Render(_)));
    }
}
