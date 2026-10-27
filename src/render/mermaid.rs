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

        let chart = self.chart_to_draw(graph)?;
        match draw(RenderRequest::svg(
            &chart,
            OperationControl::new(),
            SvgRequest::default(),
        ))? {
            RenderOutput::Svg(Some(svg)) => Ok(svg.into_parts().0),
            _ => Err(Error::Render("merman returned no SVG".into())),
        }
    }

    /// The Mermaid text, with the settings merman needs to draw it well.
    fn chart_to_draw(&self, graph: &ModuleGraph) -> Result<String> {
        // Plain SVG text for labels, not HTML in `<foreignObject>`: many viewers
        // (editors, image tools) do not draw HTML and would show empty boxes. The wide
        // wrapping keeps long paths on one line.
        Ok(format!(
            "%%{{init: {{\"htmlLabels\": false, \"flowchart\": {{\"wrappingWidth\": 600}}}}}}%%\n{}",
            self.render_to_string(graph)?
        ))
    }
}

/// Runs one merman request, turning its error into ours.
#[cfg(feature = "svg")]
fn draw(request: merman::RenderRequest<'_>) -> Result<merman::RenderOutput> {
    merman::Renderer::new()
        .render(request)
        .map_err(|err| Error::Render(err.into()))
}

/// Images are drawn on white: a transparent PNG is unreadable on a dark page, and JPEG
/// has no transparency at all.
#[cfg(any(feature = "png", feature = "jpeg"))]
fn raster_options() -> merman::svg::export::RasterOptions {
    merman::svg::export::RasterOptions {
        background: Some("white".into()),
        ..Default::default()
    }
}

#[cfg(feature = "png")]
impl Mermaid {
    /// The chart as a PNG image, made from [`render_svg`](Self::render_svg)'s picture.
    /// Needs the `png` feature.
    ///
    /// # Errors
    ///
    /// [`Error::Render`](crate::Error::Render) if merman cannot draw the chart.
    pub fn render_png(&self, graph: &ModuleGraph) -> Result<Vec<u8>> {
        use merman::{OperationControl, PngRequest, RenderOutput, RenderRequest, SvgRequest};

        let chart = self.chart_to_draw(graph)?;
        let request = PngRequest {
            svg: SvgRequest::default(),
            options: raster_options(),
        };
        match draw(RenderRequest::png(&chart, OperationControl::new(), request))? {
            RenderOutput::Png(Some(png)) => Ok(png.bytes),
            _ => Err(Error::Render("merman returned no PNG".into())),
        }
    }
}

#[cfg(feature = "jpeg")]
impl Mermaid {
    /// The chart as a JPEG image, made from [`render_svg`](Self::render_svg)'s picture.
    /// Needs the `jpeg` feature.
    ///
    /// # Errors
    ///
    /// [`Error::Render`](crate::Error::Render) if merman cannot draw the chart.
    pub fn render_jpeg(&self, graph: &ModuleGraph) -> Result<Vec<u8>> {
        use merman::{JpegRequest, OperationControl, RenderOutput, RenderRequest, SvgRequest};

        let chart = self.chart_to_draw(graph)?;
        let request = JpegRequest {
            svg: SvgRequest::default(),
            options: raster_options(),
        };
        match draw(RenderRequest::jpeg(
            &chart,
            OperationControl::new(),
            request,
        ))? {
            RenderOutput::Jpeg(Some(jpeg)) => Ok(jpeg.bytes),
            _ => Err(Error::Render("merman returned no JPEG".into())),
        }
    }
}

#[cfg(feature = "pdf")]
impl Mermaid {
    /// The chart as a one-page PDF, made from [`render_svg`](Self::render_svg)'s picture.
    /// Needs the `pdf` feature.
    ///
    /// # Errors
    ///
    /// [`Error::Render`](crate::Error::Render) if merman cannot draw the chart.
    pub fn render_pdf(&self, graph: &ModuleGraph) -> Result<Vec<u8>> {
        use merman::{OperationControl, PdfRequest, RenderOutput, RenderRequest, SvgRequest};

        let chart = self.chart_to_draw(graph)?;
        let request = PdfRequest {
            svg: SvgRequest::default(),
            options: merman::svg::export::PdfOptions {
                background: Some("white".into()),
                ..Default::default()
            },
        };
        match draw(RenderRequest::pdf(&chart, OperationControl::new(), request))? {
            RenderOutput::Pdf(Some(pdf)) => Ok(pdf.bytes),
            _ => Err(Error::Render("merman returned no PDF".into())),
        }
    }
}
