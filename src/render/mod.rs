//! Writes a graph as a [Mermaid](https://mermaid.js.org) flowchart, or as a drawing of
//! it made by [merman](https://crates.io/crates/merman).

#[cfg(any(feature = "svg", feature = "ascii"))]
mod draw;
mod mermaid;

use std::{io::Write, path::Path};

use crate::{
    error::{Error, Result},
    graph::ModuleGraph,
};

/// How to write a graph: the Mermaid flowchart, or a drawing of it.
///
/// Each drawing needs the feature of its name, like `png` for [`Png`](Self::Png).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    feature = "cli",
    derive(clap::ValueEnum, serde::Deserialize),
    serde(rename_all = "kebab-case")
)]
#[non_exhaustive]
pub enum Format {
    /// A Mermaid flowchart
    Mermaid,
    /// A picture of the Mermaid flowchart
    #[cfg(feature = "svg")]
    Svg,
    /// The Mermaid flowchart drawn with text, for the terminal
    #[cfg(feature = "ascii")]
    Ascii,
    /// A PNG image of the Mermaid flowchart
    #[cfg(feature = "png")]
    Png,
    /// A JPEG image of the Mermaid flowchart
    #[cfg(feature = "jpeg")]
    Jpeg,
    /// A PDF of the Mermaid flowchart
    #[cfg(feature = "pdf")]
    Pdf,
}

impl Format {
    /// The format a file name asks for: `.svg`, `.png`, `.jpg`, `.pdf`, or `.txt` for
    /// the text drawing. Anything else, like `.mmd`, is Mermaid.
    pub fn of(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            #[cfg(feature = "svg")]
            Some("svg") => Self::Svg,
            #[cfg(feature = "ascii")]
            Some("txt") => Self::Ascii,
            #[cfg(feature = "png")]
            Some("png") => Self::Png,
            #[cfg(feature = "jpeg")]
            Some("jpg" | "jpeg") => Self::Jpeg,
            #[cfg(feature = "pdf")]
            Some("pdf") => Self::Pdf,
            _ => Self::Mermaid,
        }
    }
}

/// Writes `graph` to `out`, like a file or the terminal, in `format`.
///
/// ```no_run
/// use oxchart::Format;
///
/// let graph = oxchart::build_graph(&oxchart::Config::new(["src/index.ts"]))?;
/// oxchart::render(&graph, Format::Mermaid, &mut std::io::stdout())?;
/// # Ok::<(), oxchart::Error>(())
/// ```
///
/// The Mermaid flowchart looks like this. Paste it in a Markdown file on GitHub, or in
/// <https://mermaid.live>, to see the chart.
///
/// ```text
/// flowchart LR
///   m0["src/a.ts"]:::file
///   m1(["react"]):::package
///   m0 -->|static| m1
///   classDef file fill:#E8F1FA,stroke:#0072B2,color:#1B1B1B
///   classDef package fill:#FFF4DE,stroke:#E69F00,color:#1B1B1B
/// ```
///
/// Files are blue boxes, packages are orange and rounded, and runtime modules are grey
/// with double sides. There is no legend: files show a path, packages a bare name and
/// runtime modules a prefix like `node:`, so the colors only help the eye.
///
/// Each arrow is labelled with its kinds of import and colored by the most important
/// one: black for `static`, blue for `re-export`, red for `require`, green for
/// `dynamic` and grey for `type`. `dynamic` and `type` arrows are dashed, because they
/// do not load when the app starts.
///
/// The Mermaid flowchart is written piece by piece, so even a big one is never held in
/// memory as a whole. Drawings are made whole first. Big graphs take longer to lay out
/// and are hard to read: past a few hundred modules, lower
/// [`Config::max_depth`](crate::Config::max_depth).
///
/// # Errors
///
/// [`Error::Render`] if `out` cannot be written to, or merman cannot draw the chart.
pub fn render(graph: &ModuleGraph, format: Format, out: &mut dyn Write) -> Result<()> {
    match format {
        Format::Mermaid => mermaid::write(graph, out).map_err(render_error),
        #[cfg(any(feature = "svg", feature = "ascii"))]
        _ => draw::draw(graph, format, out),
    }
}

/// Wraps a write error.
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

    fn rendered(graph: &ModuleGraph, format: Format) -> Vec<u8> {
        let mut out = Vec::new();
        render(graph, format, &mut out).unwrap();
        out
    }

    fn chart(graph: &ModuleGraph) -> String {
        String::from_utf8(rendered(graph, Format::Mermaid)).unwrap()
    }

    #[test]
    fn mermaid() {
        let out = chart(&graph("render-mermaid"));
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
             \x20 linkStyle 0 stroke:#0072B2,color:#0072B2\n\
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
        let out = chart(&crate::build_graph(&config).unwrap());
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
        let out = chart(&graph);
        assert!(out.contains("m0[\"a#quot;b.ts\"]"), "{out}");
    }

    #[cfg(feature = "svg")]
    #[test]
    fn mermaid_svg() {
        let svg = String::from_utf8(rendered(&graph("render-svg"), Format::Svg)).unwrap();
        assert!(svg.starts_with("<svg"), "{}", &svg[..svg.len().min(200)]);
        assert!(svg.contains("src/index.ts"));
        assert!(svg.contains("react"));
        assert!(!svg.contains("<foreignObject"));
        assert!(svg.contains("<text"));
    }

    #[cfg(feature = "ascii")]
    #[test]
    fn mermaid_ascii() {
        let drawing =
            String::from_utf8(rendered(&graph("render-mermaid-ascii"), Format::Ascii)).unwrap();
        assert!(!drawing.starts_with("flowchart"), "{drawing}");
        assert!(drawing.contains("src/index.ts"), "{drawing}");
        assert!(drawing.contains("react"), "{drawing}");
    }

    #[cfg(feature = "png")]
    #[test]
    fn mermaid_png() {
        let png = rendered(&graph("render-png"), Format::Png);
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[cfg(feature = "jpeg")]
    #[test]
    fn mermaid_jpeg() {
        let jpeg = rendered(&graph("render-jpeg"), Format::Jpeg);
        assert!(jpeg.starts_with(&[0xFF, 0xD8, 0xFF]));
    }

    #[cfg(feature = "pdf")]
    #[test]
    fn mermaid_pdf() {
        let pdf = rendered(&graph("render-pdf"), Format::Pdf);
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn format_from_extension() {
        let of = |name: &str| Format::of(Path::new(name));
        assert_eq!(of("graph.mmd"), Format::Mermaid);
        assert_eq!(of("graph"), Format::Mermaid);
        #[cfg(feature = "svg")]
        assert_eq!(of("graph.svg"), Format::Svg);
        #[cfg(feature = "ascii")]
        assert_eq!(of("graph.txt"), Format::Ascii);
        #[cfg(feature = "jpeg")]
        assert_eq!(of("graph.jpeg"), Format::Jpeg);
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
        let err = render(&graph, Format::Mermaid, &mut Closed).unwrap_err();
        assert!(matches!(err, crate::Error::Render(_)));
    }
}
