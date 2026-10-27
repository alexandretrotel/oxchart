use std::io::{self, Write};

use super::{Renderer, render_error};
#[cfg(any(feature = "svg", feature = "ascii"))]
use crate::error::Error;
use crate::{
    config::EdgeKinds,
    error::Result,
    graph::{Module, ModuleGraph},
};

/// A [Mermaid](https://mermaid.js.org) flowchart. Paste it in a Markdown file on
/// GitHub, or in <https://mermaid.live>, to see the chart.
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
/// one: black for `static`, pink for `re-export`, red for `require`, green for
/// `dynamic` and grey for `type`. `dynamic` and `type` arrows are dashed, because they
/// do not load when the app starts.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Mermaid {}

impl Renderer for Mermaid {
    fn render(&self, graph: &ModuleGraph, out: &mut dyn Write) -> Result<()> {
        write(graph, out).map_err(render_error)
    }
    /// The chart drawn with text characters by [merman](https://crates.io/crates/merman).
    /// Needs the `ascii` feature; without it, this returns an error.
    #[cfg(feature = "ascii")]
    fn render_ascii(&self, graph: &ModuleGraph) -> Result<String> {
        use merman::{AsciiRequest, OperationControl, RenderOutput, RenderRequest};

        let chart = self.render_to_string(graph)?;
        let request =
            RenderRequest::ascii(&chart, OperationControl::new(), AsciiRequest::default());
        match merman::Renderer::new().render(request) {
            Ok(RenderOutput::Ascii(Some(drawing))) => Ok(drawing.text),
            Ok(_) => Err(Error::Render("merman returned no drawing".into())),
            Err(err) => Err(Error::Render(err.into())),
        }
    }
}

/// Each kind of node: its class, its shape, and its colors.
/// Colors are from the Okabe-Ito palette, which stays readable for color-blind people.
const NODES: [(&str, &str, &str, &str); 3] = [
    (
        "file",
        "[",
        "]",
        "fill:#E8F1FA,stroke:#0072B2,color:#1B1B1B",
    ),
    (
        "package",
        "([",
        "])",
        "fill:#FFF4DE,stroke:#E69F00,color:#1B1B1B",
    ),
    (
        "runtime",
        "[[",
        "]]",
        "fill:#EEEEEE,stroke:#777777,color:#1B1B1B",
    ),
];

fn write(graph: &ModuleGraph, out: &mut dyn Write) -> io::Result<()> {
    let kind = |id| match graph.module(id) {
        Module::File(_) => 0,
        Module::Package(_) => 1,
        Module::Runtime(_) => 2,
    };
    let mut used = [false; NODES.len()];
    graph.ids().for_each(|id| used[kind(id)] = true);
    writeln!(out, "flowchart LR")?;

    // Nodes are named by id (`m0`, `m1`...): labels can hold characters Mermaid
    // does not allow in names, like `/` or `@`.
    for id in graph.ids() {
        // A `"` would end the label early; Mermaid reads `#quot;` as a quote.
        let label = graph.label(id).replace('"', "#quot;");
        let (class, open, close, ..) = NODES[kind(id)];
        writeln!(out, "  m{}{open}\"{label}\"{close}:::{class}", id.index())?;
    }

    let mut colored = Vec::new();
    for (index, edge) in graph.edges().iter().enumerate() {
        let (arrow, color) = edge_style(edge.kinds);
        let (from, to) = (edge.from.index(), edge.to.index());
        writeln!(out, "  m{from} {arrow}|{}| m{to}", edge.kinds)?;
        if let Some(color) = color {
            colored.push((index, color));
        }
    }
    for (index, color) in colored {
        writeln!(out, "  linkStyle {index} stroke:{color},color:{color}")?;
    }

    // Only the classes in use, so the chart stays short.
    for ((class, _, _, style), _) in NODES.iter().zip(used).filter(|(_, used)| *used) {
        writeln!(out, "  classDef {class} {style}")?;
    }
    Ok(())
}

/// The arrow and color of an edge, from its most important kind.
///
/// Plain imports keep the default black. Imports that do not load when the app
/// starts (`import()` and `import type`) are dashed.
fn edge_style(kinds: EdgeKinds) -> (&'static str, Option<&'static str>) {
    if kinds.contains(EdgeKinds::STATIC) {
        ("-->", None)
    } else if kinds.contains(EdgeKinds::RE_EXPORT) {
        ("-->", Some("#CC79A7"))
    } else if kinds.contains(EdgeKinds::REQUIRE) {
        ("-->", Some("#D55E00"))
    } else if kinds.contains(EdgeKinds::DYNAMIC) {
        ("-.->", Some("#009E73"))
    } else if kinds.contains(EdgeKinds::TYPE_ONLY) {
        ("-.->", Some("#999999"))
    } else {
        ("-->", None)
    }
}

#[cfg(feature = "svg")]
impl Mermaid {
    /// The chart as an SVG picture, drawn by [merman](https://crates.io/crates/merman).
    ///
    /// Needs the `svg` feature. Big graphs take longer to lay out and are hard to read:
    /// past a few hundred modules, lower [`Config::max_depth`](crate::Config::max_depth).
    ///
    /// # Errors
    ///
    /// [`Error::Render`](crate::Error::Render) if merman cannot draw the chart.
    pub fn render_svg(&self, graph: &ModuleGraph) -> Result<String> {
        use merman::{OperationControl, RenderOutput, RenderRequest, SvgRequest};

        let chart = format!(
            "%%{{init: {{\"htmlLabels\": false, \"flowchart\": {{\"wrappingWidth\": 600}}}}}}%%\n{}",
            self.render_to_string(graph)?
        );
        let request = RenderRequest::svg(&chart, OperationControl::new(), SvgRequest::default());
        match merman::Renderer::new().render(request) {
            Ok(RenderOutput::Svg(Some(svg))) => Ok(svg.into_parts().0),
            Ok(_) => Err(Error::Render("merman returned no SVG".into())),
            Err(err) => Err(Error::Render(err.into())),
        }
    }
}
