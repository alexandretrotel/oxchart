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
    /// A drawing of the graph made of plain text, to read in a terminal.
    ///
    /// Optional: renderers without one keep this default, which returns an error.
    ///
    /// # Errors
    ///
    /// [`Error::Render`] if the renderer has no text drawing, or fails to make one.
    fn render_ascii(&self, graph: &ModuleGraph) -> Result<String> {
        let _ = graph;
        Err(Error::Render("this renderer has no text drawing".into()))
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
             \x20 m0[\"src/a.ts\"]:::file\n\
             \x20 m1[\"src/b.ts\"]:::file\n\
             \x20 m2[\"src/index.ts\"]:::file\n\
             \x20 m3[\"src/lone.ts\"]:::file\n\
             \x20 m4([\"react\"]):::package\n\
             \x20 m5[[\"node:fs\"]]:::runtime\n\
             \x20 m0 -->|type, re-export| m1\n\
             \x20 m2 -->|static| m0\n\
             \x20 m2 -->|static| m4\n\
             \x20 m2 -->|static| m5\n\
             \x20 linkStyle 0 stroke:#CC79A7,color:#CC79A7\n\
             \x20 classDef file fill:#E8F1FA,stroke:#0072B2,color:#1B1B1B\n\
             \x20 classDef package fill:#FFF4DE,stroke:#E69F00,color:#1B1B1B\n\
             \x20 classDef runtime fill:#EEEEEE,stroke:#777777,color:#1B1B1B\n"
        );
    }

    #[test]
    fn mermaid_edge_styles() {
        use crate::EdgeKinds as K;
        let root = project(
            "render-edge-styles",
            &[
                (
                    "index.ts",
                    "import './a'; import('./b'); import type { T } from './c'; require('./d');",
                ),
                ("a.ts", ""),
                ("b.ts", ""),
                ("c.ts", ""),
                ("d.ts", ""),
            ],
        );
        let mut config = Config::new(["index.ts"]);
        config.root = Some(root);
        config.edge_kinds = K::ALL;
        let out = Mermaid::default()
            .render_to_string(&crate::build_graph(&config).unwrap())
            .unwrap();
        assert!(out.contains("  m4 -->|static| m0\n"), "{out}");
        assert!(out.contains("  m4 -.->|dynamic| m1\n"), "{out}");
        assert!(out.contains("  m4 -.->|type| m2\n"), "{out}");
        assert!(out.contains("  m4 -->|require| m3\n"), "{out}");
        assert!(
            out.contains("  linkStyle 1 stroke:#009E73,color:#009E73\n"),
            "{out}"
        );
        assert!(
            out.contains("  linkStyle 2 stroke:#999999,color:#999999\n"),
            "{out}"
        );
        assert!(
            out.contains("  linkStyle 3 stroke:#D55E00,color:#D55E00\n"),
            "{out}"
        );
        assert!(out.starts_with("flowchart LR\n"), "{out}");
        assert!(!out.contains("classDef runtime"), "{out}");
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
    fn ascii() {
        let graph = graph("render-ascii");
        let text = Text::default();
        assert_eq!(
            text.render_ascii(&graph).unwrap(),
            text.render_to_string(&graph).unwrap()
        );

        struct NoAscii;
        impl Renderer for NoAscii {
            fn render(&self, _: &ModuleGraph, _: &mut dyn Write) -> Result<()> {
                Ok(())
            }
        }
        assert!(matches!(
            NoAscii.render_ascii(&graph),
            Err(crate::Error::Render(_))
        ));
    }

    #[cfg(feature = "ascii")]
    #[test]
    fn mermaid_ascii() {
        let drawing = Mermaid::default()
            .render_ascii(&graph("render-mermaid-ascii"))
            .unwrap();
        assert!(!drawing.starts_with("flowchart"), "{drawing}");
        assert!(drawing.contains("src/index.ts"), "{drawing}");
        assert!(drawing.contains("react"), "{drawing}");
    }

    #[cfg(feature = "png")]
    #[test]
    fn mermaid_png() {
        let png = Mermaid::default().render_png(&graph("render-png")).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[cfg(feature = "jpeg")]
    #[test]
    fn mermaid_jpeg() {
        let jpeg = Mermaid::default()
            .render_jpeg(&graph("render-jpeg"))
            .unwrap();
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[cfg(feature = "pdf")]
    #[test]
    fn mermaid_pdf() {
        let pdf = Mermaid::default().render_pdf(&graph("render-pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
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
