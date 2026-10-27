//! `oxchart` CLI: a thin layer over the library.
//!
//! STUB. Follows https://clig.dev:
//! - `clap` derive. `-h`/`--help` with examples; `--version`.
//! - Args map 1:1 to `oxchart::Config`: positional entrypoints, `--root`, `--ext`,
//!   `--tsconfig`, `--condition`, `--include`, `--exclude`, `--max-depth`,
//!   `--externals hide|leaf|follow`, `--edges static,dynamic,type,reexport,require`.
//! - Output (graph) to stdout, or `-o FILE`. Diagnostics (unresolved imports, parse errors,
//!   "graph is large" warning) to stderr, so piping stays clean.
//! - Exit 0 on success, non-zero on error; `anyhow` with `.context()` for human messages.
//! - `-q`/`--quiet` silences warnings; `--no-color` and `NO_COLOR` respected.
//! - Validate config before any work (fail fast with a hint).

fn main() -> anyhow::Result<()> {
    todo!("parse args into Config, build graph, render")
}
