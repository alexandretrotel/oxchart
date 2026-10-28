# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-28

First release.

### Added

- `build_graph`: follows every import from one or more entrypoints and returns a
  `ModuleGraph` of files and the imports between them.
- Imports found with `oxc_parser`: `import`, `export ... from`, `import()`, global
  `require()`, `import type`, TypeScript's `import x = require()` and
  `/// <reference path>`. Each edge says how the file is imported.
- `.vue`, `.svelte` and `.astro` files are read through their `<script>` blocks and
  Astro frontmatter, with line numbers kept.
- Imports resolved with `oxc_resolver`: TypeScript extensions, the nearest
  `tsconfig.json` for path aliases, package `exports` and conditions, Node built-ins
  and runtime modules (`node:`, `bun:`, `cloudflare:`, `jsr:`...). Monorepo packages
  count as your own code.
- Entrypoints are found like imports (`src`, `src/index` and package folders work),
  and globs like `src/pages/**/*.tsx` start from every matching file.
- `Config` settings: `include`, `exclude`, `max_depth`, `externals`
  (`hide`, `leaf`, `follow`) and `edge_kinds`.
- Missing imports and syntax errors are reported without stopping the build.
- `render` feature: writes the graph as a Mermaid flowchart. `svg`, `ascii`, `png`,
  `jpeg` and `pdf` features draw it with merman.
- `oxchart` command (`cli` feature, on by default): draws the graph and opens it in a
  terminal, writes Mermaid when piped, saves with `-o`, reads `oxchart.toml`, and
  starts from `package.json` when no entrypoint is given.

[Unreleased]: https://github.com/alexandretrotel/oxchart/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/alexandretrotel/oxchart/releases/tag/v0.1.0
