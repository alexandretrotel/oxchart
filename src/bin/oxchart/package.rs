//! Entrypoints from the project's own `package.json`, used when none are given.
//!
//! Takes `exports["."]`, else `main`, else every `bin`. `dist/` is not mapped back to
//! `src/`: an unbuilt `dist/index.js` is a missing entrypoint.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use oxchart::Config;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct PackageJson {
    name: Option<String>,
    exports: Option<Value>,
    main: Option<String>,
    bin: Option<Value>,
}

/// The entrypoints `package.json` in the root names. Empty if there is no
/// `package.json`, or it names none.
pub fn entrypoints(config: &Config) -> Result<Vec<PathBuf>> {
    let root = config.root.as_deref().unwrap_or(Path::new("."));
    let path = root.join("package.json");
    let text = match fs::read_to_string(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        text => text.with_context(|| format!("cannot read {}", path.display()))?,
    };
    let package: PackageJson =
        serde_json::from_str(&text).with_context(|| format!("invalid {}", path.display()))?;

    if let (Some(name), Some(exports)) = (&package.name, &package.exports)
        && has_main_export(exports)
    {
        // A package importing itself by name goes through `exports`, with the same
        // conditions as the graph's imports.
        let mut options = config.resolve.clone();
        options.condition_names.push("import".into());
        let dir = root
            .canonicalize()
            .with_context(|| format!("cannot open root directory {}", root.display()))?;
        let resolution = oxc_resolver::Resolver::new(options)
            .resolve(dir, name)
            .with_context(|| format!("cannot find `exports[\".\"]` of {}", path.display()))?;
        return Ok(vec![resolution.into_path_buf()]);
    }
    if let Some(main) = package.main {
        return Ok(vec![main.into()]);
    }
    Ok(match package.bin {
        Some(Value::String(bin)) => vec![bin.into()],
        Some(Value::Object(bins)) => bins
            .values()
            .filter_map(Value::as_str)
            .map(PathBuf::from)
            .collect(),
        _ => Vec::new(),
    })
}

/// True when `exports` has an entry for the package itself, not only for subpaths
/// like `./utils`. Keys without a dot are conditions of that entry: `{ "import": ... }`.
fn has_main_export(exports: &Value) -> bool {
    match exports {
        Value::Object(map) => map.contains_key(".") || !map.keys().any(|k| k.starts_with('.')),
        _ => true,
    }
}
