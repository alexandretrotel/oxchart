//! Finds the imports in one file, using `oxc_parser`.
//!
//! What it does:
//! - The caller decides how to read the file with `SourceType::from_path`. If that
//!   fails, the file is not JavaScript or TypeScript (an image, a `.css`...): the graph
//!   shows it but never opens it.
//! - `import` and `export ... from` can only appear at the top level of a file, so they
//!   are read from the top-level statements without walking the whole file.
//! - An import counts as type-only (`TYPE_ONLY`) when it can only bring in types:
//!   `import type`, `export type`, or every imported name marked `type`. If even one
//!   name is real code, it is a plain import (`STATIC`).
//! - TypeScript's `import x = require('./a')` is a top-level import too (`STATIC`, or
//!   `TYPE_ONLY` with `import type`). `/// <reference path="a.d.ts" />` before the first
//!   statement only brings in types (`TYPE_ONLY`); its path is always relative to the file.
//! - `import()` and `require()` can appear anywhere, even deep inside functions, so
//!   finding them means walking the whole file. That walk is skipped when neither kind
//!   is asked for.
//! - `require()` only counts when it is the global one. A file that declares its own
//!   `require` (a variable, function or parameter) calls that instead. Checking needs
//!   `oxc_semantic`, so it only runs on files whose text contains `require`.
//! - Only fixed paths can be followed: `'./a'` or `` `./a` ``. A computed path like
//!   `require(name)` or `` import(`./${page}`) `` is reported as a warning and skipped.
//!
//! - Files that mix markup and scripts ([`COMPONENTS`]: `.vue`, `.svelte`, `.astro`) are
//!   read through their scripts only. See [`component_scripts`].
//!
//! What it returns:
//! - One entry per imported path, with every way the file imports it. Top-level imports
//!   come first, then `import()`/`require()`, each in the order they appear.
//! - Only the kinds the user asked for. The rest are dropped here, so they are never
//!   looked up on disk.
//! - Syntax errors as diagnostics. oxc recovers from small errors and keeps every
//!   import. On a serious error it gives up on the file, which then has no imports.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Declaration, Expression, IdentifierReference, ImportDeclarationSpecifier,
    ImportExpression, Statement, TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_semantic::{Scoping, SemanticBuilder, SymbolFlags};
use oxc_span::{GetSpan, SourceType, Span};

use crate::config::EdgeKinds;

/// One imported path, like `./a` or `react`, and every way the file imports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Import {
    pub specifier: String,
    pub kinds: EdgeKinds,
}

#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub imports: Vec<Import>,
    pub diagnostics: Vec<OxcDiagnostic>,
}

/// Finds the imports in `source`, keeping only the kinds in `edge_kinds`.
///
/// Never fails: anything wrong with the file is returned in `diagnostics`.
pub(crate) fn parse(
    allocator: &Allocator,
    source_type: SourceType,
    source: &str,
    edge_kinds: EdgeKinds,
) -> Parsed {
    let ret = Parser::new(allocator, source, source_type).parse();
    let mut out = Parsed {
        imports: Vec::new(),
        diagnostics: ret.diagnostics.into_vec(),
    };

    // Triple-slash directives only count before the first statement.
    let body_start = ret
        .program
        .body
        .first()
        .map_or(u32::MAX, |s| s.span().start);
    for comment in &ret.program.comments {
        if comment.is_line()
            && comment.span.end <= body_start
            && let Some(path) = reference_path(comment.content_span().source_text(source))
        {
            let specifier = if path.starts_with('.') || path.starts_with('/') {
                path.to_string()
            } else {
                format!("./{path}")
            };
            add(
                &mut out.imports,
                &specifier,
                EdgeKinds::TYPE_ONLY,
                edge_kinds,
            );
        }
    }

    for stmt in &ret.program.body {
        // `export import a = require('./a')` wraps the import in an export.
        let import_equals = match stmt {
            Statement::TSImportEqualsDeclaration(decl) => Some(decl),
            Statement::ExportDeclaration(decl) => match &decl.declaration {
                Declaration::TSImportEqualsDeclaration(decl) => Some(decl),
                _ => None,
            },
            _ => None,
        };
        let (source, kind) = match stmt {
            Statement::ImportDeclaration(decl) => {
                let all_type = decl.import_kind.is_type()
                    || decl.specifiers.as_ref().is_some_and(|s| {
                        !s.is_empty()
                            && s.iter().all(|s| {
                                matches!(s, ImportDeclarationSpecifier::ImportSpecifier(s) if s.import_kind.is_type())
                            })
                    });
                (
                    &decl.source,
                    if all_type {
                        EdgeKinds::TYPE_ONLY
                    } else {
                        EdgeKinds::STATIC
                    },
                )
            }
            Statement::ExportFromDeclaration(decl) => {
                let all_type = decl.export_kind.is_type()
                    || (!decl.specifiers.is_empty()
                        && decl.specifiers.iter().all(|s| s.export_kind.is_type()));
                (
                    &decl.source,
                    if all_type {
                        EdgeKinds::TYPE_ONLY
                    } else {
                        EdgeKinds::RE_EXPORT
                    },
                )
            }
            Statement::ExportAllDeclaration(decl) => {
                let kind = if decl.export_kind.is_type() {
                    EdgeKinds::TYPE_ONLY
                } else {
                    EdgeKinds::RE_EXPORT
                };
                (&decl.source, kind)
            }
            _ if let Some(decl) = import_equals => {
                let TSModuleReference::ExternalModuleReference(r) = &decl.module_reference else {
                    continue;
                };
                let kind = if decl.import_kind.is_type() {
                    EdgeKinds::TYPE_ONLY
                } else {
                    EdgeKinds::STATIC
                };
                (&r.expression, kind)
            }
            _ => continue,
        };
        if let Some(specifier) = source.value.as_str() {
            add(&mut out.imports, specifier, kind, edge_kinds);
        }
    }

    if edge_kinds.intersects(EdgeKinds::DYNAMIC | EdgeKinds::REQUIRE) {
        // Links each `require` to its declaration. Skipped when the text has no `require`.
        let semantic = (edge_kinds.contains(EdgeKinds::REQUIRE) && source.contains("require"))
            .then(|| SemanticBuilder::new().build(&ret.program).semantic);
        let mut walker = Walker {
            out: &mut out,
            edge_kinds,
            scoping: semantic.as_ref().map(|s| s.scoping()),
        };
        walker.visit_program(&ret.program);
    }
    out
}

/// A kind of file that mixes markup and scripts, read through its scripts only.
#[derive(Debug)]
pub(crate) struct Component {
    /// The file extension, without the dot.
    pub extension: &'static str,
    /// Whether a `---` frontmatter at the top holds TypeScript, like in Astro.
    pub frontmatter: bool,
}

/// Every [`Component`] kind. Add one here to read a new kind of file.
pub(crate) const COMPONENTS: &[Component] = &[
    Component {
        extension: "vue",
        frontmatter: false,
    },
    Component {
        extension: "svelte",
        frontmatter: false,
    },
    Component {
        extension: "astro",
        frontmatter: true,
    },
];

impl Component {
    /// The component kind of `path`, from its extension.
    pub(crate) fn of(path: &Path) -> Option<&'static Self> {
        let extension = path.extension()?;
        COMPONENTS.iter().find(|c| extension == c.extension)
    }
}

/// Keeps only the scripts of a [`Component`] file: the `<script>` blocks, plus the
/// `---` frontmatter if it has one. Every other byte becomes a space and newlines stay,
/// so offsets and line numbers still match the file.
///
/// The scripts are TypeScript if one says `lang="ts"` (always with a frontmatter), with
/// JSX if one says `lang="tsx"` or `lang="jsx"`.
pub(crate) fn component_scripts(source: &str, component: &Component) -> (SourceType, String) {
    let mut keep = Vec::new();
    let (mut ts, mut jsx) = (component.frontmatter, false);
    if component.frontmatter {
        let start = source.len() - source.trim_start().len();
        if let Some(body) = source[start..].strip_prefix("---")
            && let Some(end) = body.find("\n---")
        {
            keep.push((start + 3, start + 3 + end));
        }
    }
    // ASCII lowercasing keeps byte offsets, so `<SCRIPT>` works too.
    let lower = source.to_ascii_lowercase();
    let mut at = 0;
    while let Some(found) = lower[at..].find("<script") {
        let open = at + found;
        at = open + "<script".len();
        if !lower[at..].starts_with(|c: char| c.is_ascii_whitespace() || c == '>') {
            continue; // `<scripts>`, not a script
        }
        let Some(tag_end) = lower[open..].find('>').map(|i| open + i) else {
            break;
        };
        let tag = &lower[open..tag_end];
        if tag.ends_with('/') {
            continue; // `<script src="..." />` has no body
        }
        let Some(close) = lower[tag_end..].find("</script").map(|i| tag_end + i) else {
            break;
        };
        let lang = tag.split_once("lang=").map_or("", |(_, rest)| {
            rest.trim_start_matches(['"', '\''])
                .split(['"', '\'', ' ', '/'])
                .next()
                .unwrap_or("")
        });
        ts |= lang.starts_with("ts");
        jsx |= matches!(lang, "tsx" | "jsx");
        keep.push((tag_end + 1, close));
        at = close;
    }

    let mut bytes = source.as_bytes().to_vec();
    let mut kept = keep.iter().peekable();
    for (i, b) in bytes.iter_mut().enumerate() {
        while kept.next_if(|(_, end)| *end <= i).is_some() {}
        let inside = kept.peek().is_some_and(|(start, _)| *start <= i);
        if !inside && *b != b'\n' {
            *b = b' ';
        }
    }
    let ext = match (ts, jsx) {
        (true, true) => "tsx",
        (true, false) => "ts",
        (false, true) => "jsx",
        (false, false) => "js",
    };
    (
        SourceType::from_extension(ext).unwrap_or_default(),
        // Kept ranges start after `>` and end before `<`, so no character is cut in half.
        String::from_utf8(bytes).expect("kept ranges end on ASCII characters"),
    )
}

/// The path in a `/// <reference path="..." />` comment. `text` is the comment after `//`.
fn reference_path(text: &str) -> Option<&str> {
    let rest = text
        .strip_prefix('/')?
        .trim_start()
        .strip_prefix("<reference")?;
    let rest = &rest[rest.find("path=")? + "path=".len()..];
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let rest = &rest[1..];
    Some(&rest[..rest.find(quote)?]).filter(|p| !p.is_empty())
}

/// Adds one import. If the path is already listed, adds `kind` to it instead.
fn add(imports: &mut Vec<Import>, specifier: &str, kind: EdgeKinds, edge_kinds: EdgeKinds) {
    if !edge_kinds.contains(kind) {
        return;
    }
    match imports.iter_mut().find(|i| i.specifier == specifier) {
        Some(import) => import.kinds |= kind,
        None => imports.push(Import {
            specifier: specifier.into(),
            kinds: kind,
        }),
    }
}

/// Finds `import()` and `require()` calls anywhere in the file.
struct Walker<'o> {
    out: &'o mut Parsed,
    edge_kinds: EdgeKinds,
    scoping: Option<&'o Scoping>,
}

impl Walker<'_> {
    fn record(&mut self, arg: &Expression, span: Span, kind: EdgeKinds) {
        if !self.edge_kinds.contains(kind) {
            return;
        }
        let specifier = match arg {
            Expression::StringLiteral(s) => s.value.as_str(),
            Expression::TemplateLiteral(t) => t.single_quasi().and_then(|s| s.as_str()),
            _ => None,
        };
        match specifier {
            Some(specifier) => add(&mut self.out.imports, specifier, kind, self.edge_kinds),
            None => self.out.diagnostics.push(
                OxcDiagnostic::warn("import specifier is not a string literal; skipped")
                    .with_label(span),
            ),
        }
    }

    /// True when `ident` is not declared in the file, or only with `declare` (TypeScript's
    /// `declare const require: ...` describes the global one).
    fn is_global(&self, ident: &IdentifierReference) -> bool {
        let Some(scoping) = self.scoping else {
            return true;
        };
        ident
            .reference_id
            .get()
            .and_then(|id| scoping.get_reference(id).symbol_id())
            .is_none_or(|symbol| scoping.symbol_flags(symbol).contains(SymbolFlags::Ambient))
    }
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.record(&it.source, it.span, EdgeKinds::DYNAMIC);
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &it.callee
            && callee.name == "require"
            && self.is_global(callee)
            && let [arg] = it.arguments.as_slice()
            && let Some(arg) = arg.as_expression()
        {
            self.record(arg, it.span, EdgeKinds::REQUIRE);
        }
        walk::walk_call_expression(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: EdgeKinds = EdgeKinds::STATIC;
    const D: EdgeKinds = EdgeKinds::DYNAMIC;
    const T: EdgeKinds = EdgeKinds::TYPE_ONLY;
    const X: EdgeKinds = EdgeKinds::RE_EXPORT;
    const R: EdgeKinds = EdgeKinds::REQUIRE;

    fn run(ext: &str, source: &str, kinds: EdgeKinds) -> Parsed {
        let source_type = SourceType::from_extension(ext).unwrap();
        parse(&Allocator::default(), source_type, source, kinds)
    }

    fn imports(source: &str) -> Vec<(String, EdgeKinds)> {
        run("ts", source, EdgeKinds::ALL)
            .imports
            .into_iter()
            .map(|i| (i.specifier, i.kinds))
            .collect()
    }

    fn one(specifier: &str, kinds: EdgeKinds) -> Vec<(String, EdgeKinds)> {
        vec![(specifier.into(), kinds)]
    }

    #[test]
    fn static_imports() {
        assert_eq!(imports("import x from './a';"), one("./a", S));
        assert_eq!(imports("import './a';"), one("./a", S));
        assert_eq!(imports("import * as a from './a';"), one("./a", S));
        assert_eq!(imports("import {} from './a';"), one("./a", S));
    }

    #[test]
    fn type_only_imports() {
        assert_eq!(imports("import type { A } from './a';"), one("./a", T));
        assert_eq!(
            imports("import { type A, type B } from './a';"),
            one("./a", T)
        );
        assert_eq!(imports("import { type A, b } from './a';"), one("./a", S));
        assert_eq!(imports("export type { A } from './a';"), one("./a", T));
        assert_eq!(imports("export { type A } from './a';"), one("./a", T));
        assert_eq!(imports("export type * from './a';"), one("./a", T));
    }

    #[test]
    fn import_equals_require() {
        assert_eq!(imports("import a = require('./a');"), one("./a", S));
        assert_eq!(imports("export import a = require('./a');"), one("./a", S));
        assert_eq!(imports("import type a = require('./a');"), one("./a", T));
        assert!(imports("namespace N { export const x = 1; } import a = N.x;").is_empty());
    }

    #[test]
    fn reference_paths() {
        assert_eq!(
            imports("/// <reference path=\"./a.d.ts\" />"),
            one("./a.d.ts", T)
        );
        assert_eq!(
            imports("/// <reference path='a.d.ts'/>\nlet x;"),
            one("./a.d.ts", T)
        );
        assert_eq!(imports("/// <reference path=\"../b\" />"), one("../b", T));
        assert!(imports("/// <reference types=\"node\" />").is_empty());
        assert!(imports("// <reference path=\"./a\" />").is_empty());
        assert!(imports("/* <reference path=\"./a\" /> */").is_empty());
        assert!(imports("let x;\n/// <reference path=\"./a\" />").is_empty());
    }

    fn component(ext: &str, source: &str) -> (Vec<(String, EdgeKinds)>, Parsed) {
        let path = format!("a.{ext}");
        let (source_type, scripts) =
            component_scripts(source, Component::of(path.as_ref()).unwrap());
        assert_eq!(scripts.len(), source.len());
        assert_eq!(scripts.lines().count(), source.lines().count());
        let p = parse(&Allocator::default(), source_type, &scripts, EdgeKinds::ALL);
        let got = p
            .imports
            .iter()
            .map(|i| (i.specifier.clone(), i.kinds))
            .collect();
        (got, p)
    }

    #[test]
    fn vue_and_svelte_scripts() {
        let vue = "<template>\n  <A :x=\"import('./no')\" />é\n</template>\n\
                   <script setup lang=\"ts\">\nimport A from './A.vue';\n\
                   const x = <T,>(y: T) => y;\n</script>\n\
                   <script>\nexport default { name: 'x' };\n</script>\n\
                   <style>@import './c.css';</style>";
        let (got, p) = component("vue", vue);
        assert_eq!(got, one("./A.vue", S));
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);

        let svelte = "<SCRIPT context=\"module\">import './a';</SCRIPT>\n\
                      <script lang='ts'>import type { B } from './b';</script>\n\
                      <scripts>import './no';</scripts><script src=\"./no.js\" />\n\
                      {#if x}<p>{x}</p>{/if}";
        assert_eq!(
            component("svelte", svelte).0,
            vec![("./a".into(), S), ("./b".into(), T)]
        );
        assert_eq!(
            component("vue", "<script lang=\"tsx\">const a = <div />;</script>")
                .1
                .diagnostics,
            vec![]
        );
        assert!(component("vue", "<template><p /></template>").0.is_empty());
    }

    #[test]
    fn astro_frontmatter_and_scripts() {
        let astro = "\n---\nimport Layout from './Layout.astro';\nconst a: number = 1;\n---\n\
                     <Layout>{a}</Layout>\n<script>import './client';</script>";
        let (got, p) = component("astro", astro);
        assert_eq!(
            got,
            vec![("./Layout.astro".into(), S), ("./client".into(), S)]
        );
        assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics);
    }

    #[test]
    fn re_exports() {
        assert_eq!(imports("export * from './a';"), one("./a", X));
        assert_eq!(imports("export * as a from './a';"), one("./a", X));
        assert_eq!(imports("export { a, type B } from './a';"), one("./a", X));
        assert!(imports("const a = 1; export { a };").is_empty());
    }

    #[test]
    fn dynamic_imports() {
        assert_eq!(imports("const a = import('./a');"), one("./a", D));
        assert_eq!(imports("const a = import(`./a`);"), one("./a", D));
        assert_eq!(
            imports("async () => { await import('./a'); };"),
            one("./a", D)
        );
    }

    #[test]
    fn requires() {
        assert_eq!(imports("const a = require('./a');"), one("./a", R));
        assert_eq!(
            imports("function f() { return require(`./a`); }"),
            one("./a", R)
        );
        assert!(imports("const a = foo.require('./a');").is_empty());
        assert!(imports("const a = require('./a', 1);").is_empty());
    }

    #[test]
    fn own_require_is_not_an_import() {
        assert!(imports("function require(p) {} require('./a');").is_empty());
        assert!(imports("const require = () => {}; require('./a');").is_empty());
        assert!(imports("function f(require) { require('./a'); }").is_empty());
        assert_eq!(
            imports("function f(require) {} require('./a');"),
            one("./a", R)
        );
        assert_eq!(
            imports("declare const require: any; require('./a');"),
            one("./a", R)
        );
    }

    #[test]
    fn non_literal_specifiers_are_reported_not_imported() {
        let p = run("ts", "import(name); require(`./${name}`);", EdgeKinds::ALL);
        assert!(p.imports.is_empty());
        assert_eq!(p.diagnostics.len(), 2);
    }

    #[test]
    fn kinds_merge_per_specifier_in_source_order() {
        let src = "
            import type { A } from './a';
            import b from './b';
            export * from './a';
            const c = require('./b');
        ";
        assert_eq!(
            imports(src),
            vec![("./a".into(), T | X), ("./b".into(), S | R)]
        );
    }

    #[test]
    fn disabled_kinds_are_dropped() {
        let src = "import type { A } from './a'; import './b'; import('./c'); require('./b');";
        let p = run("ts", src, S | R);
        let got: Vec<_> = p
            .imports
            .into_iter()
            .map(|i| (i.specifier, i.kinds))
            .collect();
        assert_eq!(got, vec![("./b".into(), S | R)]);
    }

    #[test]
    fn walk_is_skipped_without_dynamic_or_require() {
        let p = run("ts", "import('./a'); require('./b'); import(x);", S);
        assert!(p.imports.is_empty());
        assert!(p.diagnostics.is_empty());
    }

    #[test]
    fn commonjs_and_jsx_files() {
        let p = run("cjs", "const a = require('./a');", EdgeKinds::ALL);
        assert_eq!(
            p.imports,
            vec![Import {
                specifier: "./a".into(),
                kinds: R
            }]
        );
        let p = run(
            "tsx",
            "import A from './a'; const x = <A />;",
            EdgeKinds::ALL,
        );
        assert_eq!(
            p.imports,
            vec![Import {
                specifier: "./a".into(),
                kinds: S
            }]
        );
    }

    #[test]
    fn recoverable_errors_keep_imports() {
        let src = "import a from './a'; export const x = 1; export { x };";
        let p = run("js", src, EdgeKinds::ALL);
        assert!(!p.diagnostics.is_empty());
        assert_eq!(
            p.imports,
            vec![Import {
                specifier: "./a".into(),
                kinds: S
            }]
        );
    }

    #[test]
    fn fatal_errors_give_diagnostics_only() {
        let p = run("ts", "import a from './a';\nconst = ;", EdgeKinds::ALL);
        assert!(!p.diagnostics.is_empty());
        assert!(p.imports.is_empty());
    }
}
