# oxchart

See which files of a JavaScript or TypeScript project import which.

Give oxchart the file your app starts from. It follows every import and prints the
graph, one line per import.

```console
$ oxchart src/index.ts
src/a.ts -> src/b.ts (re-export)
src/index.ts -> src/a.ts (static)
3 modules, 2 imports, 0 not found, 0 problems in files
```

## Install

```sh
cargo install --git https://github.com/alexandretrotel/oxchart
```

## Use

```sh
oxchart src/index.ts                    # your own files
oxchart src/main.ts --externals leaf    # also show packages, one node each
oxchart src/index.ts --max-depth 2      # stop two imports away from the start
oxchart src/index.ts --edges all        # also follow import() and import type
oxchart src/index.ts -o graph.txt       # write to a file
```

Run `oxchart --help` for every option.

By default oxchart:

- follows `import`, `export ... from` and `require()`, but not `import()` or `import type`
- hides npm packages and runtime modules (`node:fs`, `bun:sqlite`...)
- uses the nearest `tsconfig.json` of each file for path aliases like `@/utils`
- picks the Node build of packages (add `--condition browser` for a browser app)

Missing imports and syntax errors are warnings: the graph is still built.

## Settings file

Put an `oxchart.toml` in your project to stop retyping flags. oxchart looks for it
in the current folder, then in each parent folder. Flags win over the file.

```toml
entrypoints = ["src/index.ts"]
exclude = ["**/*.test.ts"]
externals = "leaf"
edges = ["static", "re-export", "require"]
max-depth = 4
```

Paths in the file start from the file's folder.

## Library

```rust
let mut config = oxchart::Config::new(["src/index.ts"]);
config.max_depth = Some(3);

let graph = oxchart::build_graph(&config)?;
for edge in graph.edges() {
    println!("{:?} -> {:?}", graph.module(edge.from), graph.module(edge.to));
}
```

To use only the library, turn off the CLI: `oxchart = { version = "0.1", default-features = false }`.

## Credits

- [oxc](https://oxc.rs) by the [VoidZero](https://voidzero.dev) team: `oxc_parser`
  reads the files and `oxc_resolver` finds the imported files. oxchart is a thin layer
  on their work.
- [merman](https://crates.io/crates/merman): renders the graph as a Mermaid chart.

## License

MIT
