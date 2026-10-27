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
//! - `import()` and `require()` can appear anywhere, even deep inside functions, so
//!   finding them means walking the whole file. That walk is skipped when neither kind
//!   is asked for.
//! - Only fixed paths can be followed: `'./a'` or `` `./a` ``. A computed path like
//!   `require(name)` or `` import(`./${page}`) `` is reported as a warning and skipped.
//!
//! What it returns:
//! - One entry per imported path, with every way the file imports it. Top-level imports
//!   come first, then `import()`/`require()`, each in the order they appear.
//! - Only the kinds the user asked for. The rest are dropped here, so they are never
//!   looked up on disk.
//! - Syntax errors as diagnostics. oxc recovers from small errors and keeps every
//!   import. On a serious error it gives up on the file, which then has no imports.

// TODO: handle `/// <reference path="...">` and TypeScript's `import x = require()`.

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    CallExpression, Expression, ImportDeclarationSpecifier, ImportExpression, Statement,
};
use oxc_ast_visit::{Visit, walk};
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};

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

    for stmt in &ret.program.body {
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
            _ => continue,
        };
        if let Some(specifier) = source.value.as_str() {
            add(&mut out.imports, specifier, kind, edge_kinds);
        }
    }

    if edge_kinds.intersects(EdgeKinds::DYNAMIC | EdgeKinds::REQUIRE) {
        let mut walker = Walker {
            out: &mut out,
            edge_kinds,
        };
        walker.visit_program(&ret.program);
    }
    out
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
}

impl<'a> Visit<'a> for Walker<'_> {
    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        self.record(&it.source, it.span, EdgeKinds::DYNAMIC);
        walk::walk_import_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        if it.callee.is_specific_id("require")
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
