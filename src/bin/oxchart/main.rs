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
    io::{self, BufWriter, IsTerminal, Write},
    path::{Path, PathBuf, absolute},
    process::ExitCode,
    time::Instant,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use oxchart::{Config, ModuleGraph};

use crate::args::{Args, Format};

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
    let start = Instant::now();
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
    // Asked for, else from the file name, else a picture for people, text for pipes.
    let format = args
        .format
        .or(format)
        .unwrap_or_else(|| match &args.output {
            Some(path) => Format::of(path),
            None if io::stdout().is_terminal() => Format::Svg,
            None => Format::Text,
        });
    if config.entrypoints.is_empty() {
        bail!(
            "no entrypoint given\n\n\
             Pass the file your app starts from, like `oxchart src/index.ts`,\n\
             or set `entrypoints` in oxchart.toml. See `oxchart --help`."
        );
    }

    let built = Instant::now();
    let graph = oxchart::build_graph(&config)?;
    let build = built.elapsed();

    let rendered = Instant::now();
    let terminal = args.output.is_none() && io::stdout().is_terminal();
    let to_open = match &args.output {
        Some(path) => {
            save(format, &graph, path)?;
            None
        }
        // A picture in a terminal: save it where the system keeps temporary files, and
        // open it. Named after the project, so each run replaces the last picture.
        None if terminal && let Some(extension) = format.picture() => {
            let name = graph.root().file_name().unwrap_or("graph".as_ref());
            let path = env::temp_dir().join(format!("oxchart-{}.{extension}", name.display()));
            save(format, &graph, &path)?;
            Some(path)
        }
        None => {
            let mut out = BufWriter::new(io::stdout().lock());
            format.write(&graph, &mut out)?;
            out.flush()?;
            None
        }
    };
    let render = rendered.elapsed();
    // Opened after timing: the viewer starting is not rendering.
    if let Some(path) = to_open {
        match opener::open(&path) {
            Ok(()) if !args.quiet => eprintln!("opened {}", path.display()),
            Ok(()) => {}
            Err(err) => eprintln!(
                "warning: cannot open {}: {err}\nOpen it yourself, or use -o to save it elsewhere.",
                path.display()
            ),
        }
    }
    if !args.quiet {
        let times = output::Times {
            total: start.elapsed(),
            build,
            render,
        };
        output::write_problems(&graph, &times, &mut io::stderr().lock())?;
    }
    Ok(())
}

fn save(format: Format, graph: &ModuleGraph, path: &Path) -> Result<()> {
    let file = File::create(path).with_context(|| format!("cannot write {}", path.display()))?;
    let mut out = BufWriter::new(file);
    format.write(graph, &mut out)?;
    out.flush()?;
    Ok(())
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|err| err.kind() == io::ErrorKind::BrokenPipe)
    })
}
