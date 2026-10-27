//! The `oxchart` command: a thin layer over the library.
//!
//! Settings come from three places. Each one wins over the one before:
//! 1. the defaults of `Config::new`,
//! 2. `oxchart.toml`, found in the current folder or a parent one (`file`),
//! 3. the flags (`args`).

mod args;
mod file;
mod output;

use std::{
    env,
    fs::File,
    io::{self, BufWriter, Write},
    path::{PathBuf, absolute},
    process::ExitCode,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use oxchart::Config;

use crate::args::Args;

fn main() -> ExitCode {
    match run(&Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        // The reader stopped early, like `oxchart ... | head`: nothing went wrong.
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<()> {
    let mut config = Config::new(Vec::<PathBuf>::new());
    let settings = match &args.config {
        _ if args.no_config => None,
        Some(path) => Some(absolute(path)?),
        None => file::find(&env::current_dir()?),
    };
    let mut format = None;
    if let Some(path) = settings {
        let settings = file::load(&path)?;
        settings.apply(&path, &mut config)?;
        format = settings.format;
    }
    args.apply(&mut config)?;
    let renderer = args.format.or(format).unwrap_or_default().renderer();
    if config.entrypoints.is_empty() {
        bail!(
            "no entrypoint given\n\n\
             Pass the file your app starts from, like `oxchart src/index.ts`,\n\
             or set `entrypoints` in oxchart.toml. See `oxchart --help`."
        );
    }

    let graph = oxchart::build_graph(&config)?;

    match &args.output {
        Some(path) => {
            let file =
                File::create(path).with_context(|| format!("cannot write {}", path.display()))?;
            let mut out = BufWriter::new(file);
            renderer.render(&graph, &mut out)?;
            out.flush()?;
        }
        None => {
            let mut out = BufWriter::new(io::stdout().lock());
            renderer.render(&graph, &mut out)?;
            out.flush()?;
        }
    }
    if !args.quiet {
        output::write_problems(&graph, &mut io::stderr().lock())?;
    }
    Ok(())
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::BrokenPipe)
    })
}
