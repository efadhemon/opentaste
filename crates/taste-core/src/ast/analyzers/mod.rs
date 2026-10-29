//! Per-file statistical detectors.
//!
//! Each detector returns raw counts for one file. Nothing in this module
//! decides that a count is a rule; that threshold lives in [`super::promote`].

mod arch;
mod functions;
mod imports;
mod types;

pub(crate) use arch::{decorator_name, is_custom_error, is_framework_decorator};

use std::ops::AddAssign;

use tree_sitter::Tree;

use super::query::{self, ScriptQuery};
use super::SupportedLanguage;

/// One side of a binary choice, plus how many files contributed to it.
///
/// `files` counts files, not occurrences. A file that contains both sides
/// contributes one file. The promotion floor is about files, so folding
/// tallies has to keep that number instead of reconstructing it from totals.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Count {
    /// Occurrences of the first alternative, such as alias imports.
    pub left: u32,
    /// Occurrences of the second alternative, such as deep relative imports.
    pub right: u32,
    /// Files that contained at least one occurrence of either side.
    pub files: u32,
}

impl Count {
    pub(crate) fn from_sides(left: u32, right: u32) -> Self {
        Count {
            left,
            right,
            files: u32::from(left + right > 0),
        }
    }
}

impl AddAssign for Count {
    fn add_assign(&mut self, rhs: Self) {
        self.left = self.left.saturating_add(rhs.left);
        self.right = self.right.saturating_add(rhs.right);
        self.files = self.files.saturating_add(rhs.files);
    }
}

/// Raw counts for every dimension the cold-start scanner measures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// `left` is `@/` or `~/`. `right` is a deep relative path (`../../`).
    pub import_paths: Count,
    /// `left` is a named export. `right` is a default export.
    pub exports: Count,
    /// `left` is `interface`. `right` is `type`. JavaScript never contributes.
    pub type_constructs: Count,
    /// `left` is `function foo()`. `right` is `const foo = () =>`.
    pub function_syntax: Count,
    /// `left` is a function with an early-return guard. `right` is one with `if/else`.
    pub control_flow: Count,
    /// `left` is a project-specific decorator. `right` is a framework decorator.
    pub decorators: Count,
    /// `left` is a custom error class or throw. `right` is bare `Error` or `HttpException`.
    pub errors: Count,
}

impl AddAssign for Tally {
    fn add_assign(&mut self, rhs: Self) {
        self.import_paths += rhs.import_paths;
        self.exports += rhs.exports;
        self.type_constructs += rhs.type_constructs;
        self.function_syntax += rhs.function_syntax;
        self.control_flow += rhs.control_flow;
        self.decorators += rhs.decorators;
        self.errors += rhs.errors;
    }
}

/// One measured choice, labelled for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimension {
    pub name: &'static str,
    /// What [`Count::left`] counts.
    pub left: &'static str,
    /// What [`Count::right`] counts.
    pub right: &'static str,
    pub count: Count,
}

impl Tally {
    /// Every dimension with human-readable labels, in a stable order, so
    /// `taste status` can show *why* a rule was or was not inferred.
    pub fn dimensions(&self) -> [Dimension; 7] {
        let dimension = |name, left, right, count| Dimension {
            name,
            left,
            right,
            count,
        };
        [
            dimension(
                "imports",
                "path aliases",
                "deep relative",
                self.import_paths,
            ),
            dimension("exports", "named", "default", self.exports),
            dimension(
                "object types",
                "interface",
                "type alias",
                self.type_constructs,
            ),
            dimension(
                "functions",
                "declarations",
                "const arrows",
                self.function_syntax,
            ),
            dimension(
                "control flow",
                "guard clauses",
                "if/else",
                self.control_flow,
            ),
            dimension("decorators", "project", "framework", self.decorators),
            dimension("errors", "custom", "bare Error", self.errors),
        ]
    }
}

/// Persist a per-file tally in `file_snapshots.ast_signature`.
///
/// Incremental rescans skip the parse but still have to fold the file into the
/// repository ratio. The signature is that file's counts, not a second copy of
/// the source.
pub(crate) fn encode_tally(tally: &Tally) -> String {
    fn part(count: Count) -> String {
        format!("{},{},{}", count.left, count.right, count.files)
    }
    format!(
        "tally1;{};{};{};{};{};{};{}",
        part(tally.import_paths),
        part(tally.exports),
        part(tally.type_constructs),
        part(tally.function_syntax),
        part(tally.control_flow),
        part(tally.decorators),
        part(tally.errors),
    )
}

pub(crate) fn decode_tally(signature: &str) -> Option<Tally> {
    let rest = signature.strip_prefix("tally1;")?;
    let mut parts = rest.split(';');
    Some(Tally {
        import_paths: decode_count(parts.next()?)?,
        exports: decode_count(parts.next()?)?,
        type_constructs: decode_count(parts.next()?)?,
        function_syntax: decode_count(parts.next()?)?,
        control_flow: decode_count(parts.next()?)?,
        decorators: decode_count(parts.next()?)?,
        errors: decode_count(parts.next()?)?,
    })
}

fn decode_count(raw: &str) -> Option<Count> {
    let mut nums = raw.split(',');
    Some(Count {
        left: nums.next()?.parse().ok()?,
        right: nums.next()?.parse().ok()?,
        files: nums.next()?.parse().ok()?,
    })
}

/// Count every dimension in one source file.
///
/// An unparseable buffer contributes nothing. Returning an empty tally, rather
/// than failing the scan, keeps one broken file from discarding the rest.
pub fn analyze_source(lang: SupportedLanguage, source: &str) -> Tally {
    let Some(tree) = super::parse(lang, source) else {
        return Tally::default();
    };
    if !lang.is_script() {
        return Tally::default();
    }
    tally_tree(lang, &tree, source)
}

pub(crate) fn tally_tree(lang: SupportedLanguage, tree: &Tree, source: &str) -> Tally {
    Tally {
        import_paths: imports::import_paths(lang, tree, source),
        exports: imports::exports(lang, tree, source),
        type_constructs: types::type_constructs(lang, tree, source),
        function_syntax: functions::function_syntax(lang, tree, source),
        control_flow: functions::control_flow(lang, tree, source),
        decorators: arch::decorators(lang, tree, source),
        errors: arch::errors(lang, tree, source),
    }
}

pub(crate) fn run<'a>(
    query: &'static ScriptQuery,
    lang: SupportedLanguage,
    tree: &'a Tree,
    source: &str,
) -> Vec<(usize, tree_sitter::Node<'a>)> {
    match query.get(lang) {
        Some(compiled) => query::captures(compiled, tree, source),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tally(lang: SupportedLanguage, source: &str) -> Tally {
        analyze_source(lang, source)
    }

    #[test]
    fn import_paths_count_aliases_and_deep_relatives_only() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            import { a } from "@/a";
            import { b } from "~/b";
            import { c } from "../../c";
            import { d } from "../d";
            import { e } from "./e";
            "#,
        );
        assert_eq!(got.import_paths, Count::from_sides(2, 1));
    }

    #[test]
    fn exports_distinguish_named_from_default() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            export function a() { return 1; }
            export default function b() { return 2; }
            export { c };
            "#,
        );
        assert_eq!(got.exports, Count::from_sides(2, 1));
    }

    #[test]
    fn javascript_files_do_not_contribute_to_the_type_ratio() {
        let got = tally(
            SupportedLanguage::JavaScript,
            r#"
            interface A { id: string }
            type B = { id: string };
            "#,
        );
        assert_eq!(got.type_constructs, Count::default());
    }

    #[test]
    fn type_constructs_count_interfaces_against_aliases() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            interface A { id: string }
            interface C { name: string }
            type B = { id: string };
            "#,
        );
        assert_eq!(got.type_constructs, Count::from_sides(2, 1));
    }

    #[test]
    fn aliases_an_interface_could_not_express_do_not_count() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            interface A { id: string }
            type Status = "open" | "closed";
            type Handler = (event: string) => void;
            type Keys = keyof A;
            type Partial2 = Partial<A>;
            "#,
        );
        assert_eq!(got.type_constructs, Count::from_sides(1, 0));
    }

    #[test]
    fn function_syntax_counts_declarations_against_const_arrows() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            function foo() { return 1; }
            const bar = () => 1;
            const baz = () => { return 2; };
            const other = function () { return 3; };
            "#,
        );
        assert_eq!(got.function_syntax, Count::from_sides(1, 2));
    }

    #[test]
    fn control_flow_counts_guards_against_else_branches() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            function guarded(x) {
              if (!x) return;
              return x;
            }
            function nested(x) {
              if (x) { return x; } else { return 0; }
            }
            "#,
        );
        assert_eq!(got.control_flow, Count::from_sides(1, 1));
    }

    #[test]
    fn decorators_separate_framework_names_from_project_ones() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            @Entity()
            class A {}
            @AutoEntity()
            class B {}
            "#,
        );
        assert_eq!(got.decorators, Count::from_sides(1, 1),);
    }

    #[test]
    fn errors_separate_custom_classes_from_bare_error() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            class DomainError extends Error {}
            throw new Error("x");
            throw new DomainError("y");
            throw new TypeError("z");
            "#,
        );
        assert_eq!(got.errors, Count::from_sides(2, 1));
    }

    #[test]
    fn nestjs_built_in_exceptions_are_not_custom_errors() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            throw new NotFoundException();
            throw new BadRequestException("x");
            throw new AggregateError([]);
            "#,
        );
        assert_eq!(got.errors, Count::default());
    }

    #[test]
    fn an_exception_imported_from_a_package_is_not_custom() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            import { AxiosError } from "axios";
            import { OrderError } from "@/orders/errors";
            throw new AxiosError("x");
            throw new OrderError("y");
            "#,
        );
        assert_eq!(got.errors, Count::from_sides(1, 0));
    }

    #[test]
    fn everyday_framework_decorators_are_not_project_specific() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            class A {
              @IsNotEmpty() a;
              @InjectRepository(X) b;
              @CreateDateColumn() c;
              @HttpCode(200) d() {}
            }
            "#,
        );
        assert_eq!(got.decorators, Count::from_sides(0, 4));
    }

    #[test]
    fn import_origin_decides_a_decorator_before_its_name() {
        let got = tally(
            SupportedLanguage::TypeScript,
            r#"
            import { Audited } from "some-audit-lib";
            import { Entity } from "@/db/entity";
            @Audited()
            @Entity()
            class A {}
            "#,
        );
        assert_eq!(got.decorators, Count::from_sides(1, 1));
    }

    #[test]
    fn a_rust_file_contributes_no_script_counts() {
        let got = tally(SupportedLanguage::Rust, "fn main() {}\n");
        assert_eq!(got, Tally::default());
    }
}
