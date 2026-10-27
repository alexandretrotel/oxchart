use std::io::{self, Write};

use super::{Renderer, render_error};
use crate::{
    error::Result,
    graph::{Module, ModuleGraph},
};

/// A [Mermaid](https://mermaid.js.org) flowchart. Paste it in a Markdown file on
/// GitHub, or in <https://mermaid.live>, to see the chart.
///
/// ```text
/// flowchart LR
///   m0["src/a.ts"]
///   m1(["react"])
///   m2[["node:fs"]]
///   m0 -->|static| m1
/// ```
///
/// Files are boxes, packages are rounded, and runtime modules have double sides. Each
/// arrow is labelled with the kinds of import.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Mermaid {}

impl Renderer for Mermaid {
    fn render(&self, graph: &ModuleGraph, out: &mut dyn Write) -> Result<()> {
        write(graph, out).map_err(render_error)
    }
}

fn write(graph: &ModuleGraph, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "flowchart LR")?;
    // Nodes are named by id (`m0`, `m1`...): labels can hold characters Mermaid
    // does not allow in names, like `/` or `@`.
    for id in graph.ids() {
        // A `"` would end the label early; Mermaid reads `#quot;` as a quote.
        let label = graph.label(id).replace('"', "#quot;");
        let index = id.index();
        match graph.module(id) {
            Module::File(_) => writeln!(out, "  m{index}[\"{label}\"]")?,
            Module::Package(_) => writeln!(out, "  m{index}([\"{label}\"])")?,
            Module::Runtime(_) => writeln!(out, "  m{index}[[\"{label}\"]]")?,
        }
    }
    for edge in graph.edges() {
        let (from, to) = (edge.from.index(), edge.to.index());
        writeln!(out, "  m{from} -->|{}| m{to}", edge.kinds)?;
    }
    Ok(())
}
