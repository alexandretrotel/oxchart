//! Drawings of the Mermaid flowchart, made by [merman](https://crates.io/crates/merman).

use std::io::Write;

use merman::{OperationControl, RenderOutput, RenderRequest};

use super::{Format, mermaid, render_error};
use crate::{
    error::{Error, Result},
    graph::ModuleGraph,
};

/// Plain SVG text for labels, not HTML in `<foreignObject>`: many viewers (editors,
/// image tools) do not draw HTML and would show empty boxes. The wide wrapping keeps
/// long paths on one line.
const SETTINGS: &str =
    "%%{init: {\"htmlLabels\": false, \"flowchart\": {\"wrappingWidth\": 600}}}%%\n";

/// Draws `graph` in `format`, any format but [`Format::Mermaid`].
pub(super) fn draw(graph: &ModuleGraph, format: Format, out: &mut dyn Write) -> Result<()> {
    let mut chart = SETTINGS.as_bytes().to_vec();
    mermaid::write(graph, &mut chart).map_err(render_error)?;
    let chart = String::from_utf8(chart).map_err(|err| Error::Render(err.into()))?;

    let control = OperationControl::new();
    let request = match format {
        Format::Mermaid => unreachable!("Mermaid is written, not drawn"),
        #[cfg(feature = "ascii")]
        Format::Ascii => RenderRequest::ascii(&chart, control, merman::AsciiRequest::default()),
        #[cfg(feature = "svg")]
        Format::Svg => RenderRequest::svg(&chart, control, merman::SvgRequest::default()),
        #[cfg(feature = "png")]
        Format::Png => RenderRequest::png(
            &chart,
            control,
            merman::PngRequest {
                svg: merman::SvgRequest::default(),
                options: raster_options(),
            },
        ),
        #[cfg(feature = "jpeg")]
        Format::Jpeg => RenderRequest::jpeg(
            &chart,
            control,
            merman::JpegRequest {
                svg: merman::SvgRequest::default(),
                options: raster_options(),
            },
        ),
        #[cfg(feature = "pdf")]
        Format::Pdf => RenderRequest::pdf(
            &chart,
            control,
            merman::PdfRequest {
                svg: merman::SvgRequest::default(),
                options: merman::svg::export::PdfOptions {
                    background: Some("white".into()),
                    ..Default::default()
                },
            },
        ),
    };

    let drawing = match merman::Renderer::new()
        .render(request)
        .map_err(|err| Error::Render(err.into()))?
    {
        #[cfg(feature = "ascii")]
        RenderOutput::Ascii(Some(drawing)) => drawing.text.into_bytes(),
        #[cfg(feature = "svg")]
        RenderOutput::Svg(Some(svg)) => svg.into_parts().0.into_bytes(),
        #[cfg(feature = "png")]
        RenderOutput::Png(Some(png)) => png.bytes,
        #[cfg(feature = "jpeg")]
        RenderOutput::Jpeg(Some(jpeg)) => jpeg.bytes,
        #[cfg(feature = "pdf")]
        RenderOutput::Pdf(Some(pdf)) => pdf.bytes,
        _ => return Err(Error::Render("merman returned no drawing".into())),
    };
    out.write_all(&drawing).map_err(render_error)
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
