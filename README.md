# oxchart

See which files of a JavaScript or TypeScript project import which.

Give oxchart the file your app starts from. It follows every import, draws the graph
as a picture and opens it.

```console
$ oxchart src/index.ts
opened /tmp/oxchart-my-app.svg
3 modules, 2 imports, 0 not found, 0 problems in files
```

Here is [zustand](https://github.com/pmndrs/zustand) v5.0.16, drawn by oxchart:

![Import graph of zustand v5.0.16](assets/zustand.svg)

Made with this command, run in the zustand repository:

```sh
oxchart src/index.ts src/vanilla.ts src/middleware.ts src/middleware/immer.ts \
  src/shallow.ts src/traditional.ts --externals leaf -o zustand.svg
```

When the output is piped, it writes the [Mermaid](https://mermaid.js.org) chart
instead, to paste on GitHub or in [mermaid.live](https://mermaid.live):

```console
$ oxchart src/index.ts | head -3
flowchart LR
  m0["src/a.ts"]:::file
  m1["src/b.ts"]:::file
```

## Install

```sh
cargo install --git https://github.com/alexandretrotel/oxchart
```

## Use

```sh
oxchart src/index.ts                    # open a picture of your own files
oxchart src/main.ts --externals leaf    # also show packages, one node each
oxchart src/index.ts --max-depth 2      # stop two imports away from the start
oxchart src/index.ts --edges all        # also follow import() and import type
oxchart src/index.ts -o graph.svg       # save the picture instead of opening it
oxchart src/index.ts -o graph.png       # also .jpg and .pdf
oxchart src/index.ts -o graph.mmd       # Mermaid chart, for GitHub or mermaid.live
oxchart src/index.ts --format ascii     # draw the chart in the terminal
```

With `-o`, the file extension picks the format: `.svg`, `.png`, `.jpg`, `.pdf`, or
`.txt` for the text drawing. Anything else, like `.mmd`, is the Mermaid chart.
`--format svg|png|jpeg|pdf|mermaid|ascii` always wins.

Run `oxchart --help` for every option.

By default oxchart:

- follows `import`, `export ... from` and `require()`, but not `import()` or `import type`
- hides npm packages and runtime modules (`node:fs`, `bun:sqlite`...)
- uses the nearest `tsconfig.json` of each file for path aliases like `@/utils`
- picks the Node build of packages (add `--condition-names browser` for a browser app)

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

// The Mermaid chart, or a drawing of it like `Format::Svg`.
oxchart::render(&graph, oxchart::Format::Mermaid, &mut std::io::stdout())?;
```

Each drawing needs the feature of its name: `svg`, `ascii`, `png`, `jpeg` or `pdf`.

## Credits

- [oxc](https://oxc.rs) by the [VoidZero](https://voidzero.dev) team: `oxc_parser`
  reads the files and `oxc_resolver` finds the imported files. oxchart is a thin layer
  on their work.
- [merman](https://crates.io/crates/merman): oxchart writes a Mermaid chart, and
  merman draws it in every other format: SVG, PNG, JPEG, PDF and text.

## License

MIT
