//! Warnings and the summary, printed to stderr after the graph.
//!
//! The graph itself is written by a renderer from the library.

use std::io::{self, Write};

use oxchart::ModuleGraph;

/// Above this many modules, a chart is hard to read.
const LARGE: usize = 500;

/// Imports that lead nowhere, problems inside files, and a one-line summary.
pub fn write_problems(graph: &ModuleGraph, out: &mut impl Write) -> io::Result<()> {
    for unresolved in graph.unresolved() {
        writeln!(
            out,
            "warning: {}: cannot find `{}`",
            graph.label(unresolved.from),
            unresolved.specifier
        )?;
    }
    for diagnostic in graph.diagnostics() {
        let file = graph.label(diagnostic.module);
        match diagnostic.line {
            Some(line) => writeln!(out, "warning: {file}:{line}: {}", diagnostic.message)?,
            None => writeln!(out, "warning: {file}: {}", diagnostic.message)?,
        }
    }
    let modules = graph.modules().len();
    if modules > LARGE {
        writeln!(
            out,
            "warning: {modules} modules is a lot to read; try --max-depth or --exclude"
        )?;
    }
    writeln!(
        out,
        "{modules} modules, {} imports, {} not found, {} problems in files",
        graph.edges().len(),
        graph.unresolved().len(),
        graph.diagnostics().len()
    )
}
