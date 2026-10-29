//! Fixture-driven checks for the structural detectors.
//!
//! Each detector has a positive edit and a negative edit that must not fire.
//! A wrong rule is worse than a missing one, so the negative case is the
//! assertion that matters when the two disagree.

use std::fs;
use std::path::Path;

use taste_core::analyze;
use taste_core::models::RuleSource;

fn pair(detector: &str, negative: bool) -> (String, String) {
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(detector);
    if negative {
        dir.push("negative");
    }
    (
        fs::read_to_string(dir.join("before.ts")).unwrap(),
        fs::read_to_string(dir.join("after.ts")).unwrap(),
    )
}

fn fires(detector: &str, path: &str, needle: &str) {
    let (before, after) = pair(detector, false);
    let found = analyze(&before, &after, Path::new(path)).unwrap();
    assert_eq!(found.len(), 1, "{detector} positive: {found:?}");
    assert_eq!(found[0].source, RuleSource::AstDiff);
    assert!(
        found[0].instruction.contains(needle),
        "{detector}: {}",
        found[0].instruction
    );
    assert!(found[0].anti_pattern.is_some());
    assert!(found[0].preferred_pattern.is_some());
    assert_ne!(found[0].scope, "**/*");

    let (before, after) = pair(detector, true);
    let found = analyze(&before, &after, Path::new(path)).unwrap();
    assert!(
        found.is_empty(),
        "{detector} negative must not fire: {found:?}"
    );
}

#[test]
fn decorator_swap_fires_only_when_the_decorator_name_changes() {
    fires("decorator-swap", "src/users/user.entity.ts", "AutoEntity");
}

#[test]
fn import_rewrite_fires_only_when_relative_becomes_alias() {
    fires("import-rewrite", "src/app.ts", "path aliases");
}

#[test]
fn logger_replacement_fires_only_when_console_becomes_a_logger() {
    fires("logger-replacement", "src/app.ts", "logger");
}

#[test]
fn type_swap_fires_only_when_interface_becomes_type() {
    fires("type-swap", "src/app.ts", "type");
}

#[test]
fn function_syntax_fires_only_when_a_declaration_becomes_an_arrow() {
    fires("function-syntax", "src/app.ts", "arrow");
}

#[test]
fn error_class_fires_only_when_bare_error_becomes_a_custom_class() {
    fires("error-class", "src/app.ts", "custom error");
}

#[test]
fn control_flow_fires_only_when_else_becomes_a_guard() {
    fires("control-flow", "src/app.ts", "guard");
}

#[test]
fn swapping_one_framework_decorator_for_another_is_not_a_taste() {
    let before = "@Controller()\nexport class A {\n  @Get()\n  list() {}\n}\n";
    let after = "@Controller()\nexport class A {\n  @Post()\n  list() {}\n}\n";
    let found = analyze(before, after, Path::new("src/a/a.controller.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_decorator_imported_from_a_package_is_not_project_specific() {
    let before = "import { Entity } from \"typeorm\";\n@Entity()\nexport class User {}\n";
    let after = "import { Schema } from \"@nestjs/mongoose\";\n@Schema()\nexport class User {}\n";
    let found = analyze(before, after, Path::new("src/users/user.entity.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_project_decorator_replacing_one_from_a_package_is_a_taste() {
    let before = "import { Table } from \"some-orm\";\n@Table()\nexport class User {}\n";
    let after =
        "import { AutoTable } from \"@/db/auto-table\";\n@AutoTable()\nexport class User {}\n";
    let found = analyze(before, after, Path::new("src/users/user.entity.ts")).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].instruction.contains("AutoTable"), "{found:?}");
}

#[test]
fn bare_error_becoming_a_framework_exception_is_not_a_custom_error_taste() {
    let before = "export function load() {\n  throw new Error(\"missing\");\n}\n";
    let nest = "export function load() {\n  throw new NotFoundException(\"missing\");\n}\n";
    let found = analyze(before, nest, Path::new("src/users/user.service.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");

    let package = "import { AxiosError } from \"axios\";\nexport function load() {\n  throw new AxiosError(\"missing\");\n}\n";
    let found = analyze(before, package, Path::new("src/users/user.service.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_alias_becoming_a_sibling_import_is_not_a_relative_import_taste() {
    let found = analyze(
        "import { a } from \"@/users/a\";\n",
        "import { a } from \"./a\";\n",
        Path::new("src/users/b.ts"),
    )
    .unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn an_alias_becoming_a_deep_relative_import_uses_the_scanner_instruction() {
    let found = analyze(
        "import { a } from \"@/lib/a\";\n",
        "import { a } from \"../../lib/a\";\n",
        Path::new("src/users/b.ts"),
    )
    .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].instruction, taste_core::ast::PREFER_DEEP_RELATIVE);
    assert_eq!(
        taste_core::ast::opposite_instruction(&found[0].instruction),
        Some(taste_core::ast::PREFER_PATH_ALIASES)
    );
}

#[test]
fn a_class_method_refactored_into_a_guard_clause_is_detected() {
    let before = "export class UserService {\n  label(user) {\n    if (user) {\n      return user.name;\n    } else {\n      return \"guest\";\n    }\n  }\n}\n";
    let after = "export class UserService {\n  label(user) {\n    if (!user) return \"guest\";\n    return user.name;\n  }\n}\n";
    let found = analyze(before, after, Path::new("src/users/user.service.ts")).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].instruction.contains("guard"), "{found:?}");
}

#[test]
fn a_const_arrow_refactored_into_a_guard_clause_is_detected() {
    let before = "const label = (user) => {\n  if (user) {\n    return user.name;\n  } else {\n    return \"guest\";\n  }\n};\n";
    let after =
        "const label = (user) => {\n  if (!user) return \"guest\";\n  return user.name;\n};\n";
    let found = analyze(before, after, Path::new("src/label.ts")).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn same_named_methods_in_different_classes_do_not_pair() {
    // `A.load` loses its else without gaining a guard, while `B.load` gains a
    // guard it never had an else for. Keyed by bare name, the last `load` in
    // each buffer wins, so A's else was paired with B's guard.
    let before = "class B {\n  load(x) {\n    return x || 0;\n  }\n}\nclass A {\n  load(x) {\n    if (x) {\n      return 1;\n    } else {\n      return 2;\n    }\n  }\n}\n";
    let after = "class A {\n  load(x) {\n    return x ? 1 : 2;\n  }\n}\nclass B {\n  load(x) {\n    if (!x) return 0;\n    return x;\n  }\n}\n";
    let found = analyze(before, after, Path::new("src/a.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn identical_throws_in_different_functions_do_not_pair() {
    // `a` drops its bare throw and `b` rewrites the line holding its custom
    // one. Keyed by arguments alone, both were `()` and paired with each other.
    let before =
        "function b(x) {\n  throw new DomainError();\n}\nfunction a() {\n  throw new Error();\n}\n";
    let after = "function a() {\n  return null;\n}\nfunction b(x) {\n  if (x) throw new DomainError();\n}\n";
    let found = analyze(before, after, Path::new("src/a.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_deleted_console_call_does_not_hide_a_replaced_one() {
    let before = "console.log('a');\nconsole.log('b');\n";
    let after = "foo();\nlogger.info('b');\n";
    let found = analyze(before, after, Path::new("src/app.ts")).unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].instruction, taste_core::diff::PREFER_LOGGER);
}
