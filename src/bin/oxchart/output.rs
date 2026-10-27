//! What the command prints.
//!
//! The graph goes to stdout (or `-o FILE`), one line per import, so it can be piped
//! to `grep` or `sort`. Warnings and the summary go to stderr.

use std::io::{self, Write};

use oxchart::{Module, ModuleGraph};

/// Above this many modules, a chart is hard to read.
const LARGE: usize = 500;

/// One line per import: `src/a.ts -> src/b.ts (static, re-export)`. A module that
/// imports nothing and is imported by nothing gets a line with just its name.
pub fn write_graph(graph: &ModuleGraph, out: &mut impl Write) -> io::Result<()> {
    let mut linked = vec![false; graph.modules().len()];
    for edge in graph.edges() {
        linked[edge.from.index()] = true;
        linked[edge.to.index()] = true;
        writeln!(
            out,
            "{} -> {} ({})",
            name(graph, graph.module(edge.from)),
            name(graph, graph.module(edge.to)),
            edge.kinds
        )?;
    }
    for (module, _) in graph
        .modules()
        .iter()
        .zip(linked)
        .filter(|(_, linked)| !linked)
    {
        writeln!(out, "{}", name(graph, module))?;
    }
    Ok(())
}

/// Imports that lead nowhere, problems inside files, and a one-line summary.
pub fn write_problems(graph: &ModuleGraph, out: &mut impl Write) -> io::Result<()> {
    for unresolved in graph.unresolved() {
        writeln!(
            out,
            "warning: {}: cannot find `{}`",
            name(graph, graph.module(unresolved.from)),
            unresolved.specifier
        )?;
    }
    for diagnostic in graph.diagnostics() {
        let file = name(graph, graph.module(diagnostic.module));
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

/// A short name: the path from the root for files, the name for the rest.
fn name(graph: &ModuleGraph, module: &Module) -> String {
    match module {
        Module::File(path) => path
            .strip_prefix(graph.root())
            .unwrap_or(path)
            .display()
            .to_string(),
        Module::Package(name) | Module::Runtime(name) => name.clone(),
        other => format!("{other:?}"),
    }
}
