use std::io::{self, Write};

use super::{Renderer, render_error};
use crate::{error::Result, graph::ModuleGraph};

/// One line per import, easy to read and to search with `grep`:
///
/// ```text
/// src/a.ts -> src/b.ts (type, re-export)
/// src/index.ts -> src/a.ts (static)
/// src/lone.ts
/// ```
///
/// A module that imports nothing and is imported by nothing gets a line with just its
/// name, so no module is left out.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Text {}

impl Renderer for Text {
    fn render(&self, graph: &ModuleGraph, out: &mut dyn Write) -> Result<()> {
        write(graph, out).map_err(render_error)
    }
}

fn write(graph: &ModuleGraph, out: &mut dyn Write) -> io::Result<()> {
    let mut linked = vec![false; graph.modules().len()];
    for edge in graph.edges() {
        linked[edge.from.index()] = true;
        linked[edge.to.index()] = true;
        let (from, to) = (graph.label(edge.from), graph.label(edge.to));
        writeln!(out, "{from} -> {to} ({})", edge.kinds)?;
    }
    for id in graph.ids().filter(|id| !linked[id.index()]) {
        writeln!(out, "{}", graph.label(id))?;
    }
    Ok(())
}
