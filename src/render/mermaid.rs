//! The Mermaid flowchart: the text every other format is drawn from.

use std::io::{self, Write};

use crate::{
    config::EdgeKinds,
    graph::{Module, ModuleGraph},
};

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

pub(super) fn write(graph: &ModuleGraph, out: &mut dyn Write) -> io::Result<()> {
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
/// `import` and `export ... from` keep the default black, and `import` is thick so
/// the main imports stand out. `import type` is dashed: it is gone once compiled.
fn edge_style(kinds: EdgeKinds) -> (&'static str, Option<&'static str>) {
    if kinds.contains(EdgeKinds::STATIC) {
        ("==>", None)
    } else if kinds.contains(EdgeKinds::RE_EXPORT) {
        ("-->", None)
    } else if kinds.contains(EdgeKinds::REQUIRE) {
        ("-->", Some("#D55E00"))
    } else if kinds.contains(EdgeKinds::DYNAMIC) {
        ("-->", Some("#009E73"))
    } else if kinds.contains(EdgeKinds::TYPE_ONLY) {
        ("-.->", Some("#999999"))
    } else {
        ("-->", None)
    }
}
