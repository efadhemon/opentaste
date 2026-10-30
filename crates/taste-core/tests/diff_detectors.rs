//! Fixture-driven checks for the structural detectors.
//!
//! Each detector has a positive edit and a negative edit that must not fire.
//! A wrong rule is worse than a missing one, so the negative case is the
//! assertion that matters when the two disagree.

use std::fs;
use std::path::Path;

use taste_core::analyze;
use taste_core::models::RuleSource;

/// `before.<ext>` and `after.<ext>` under `tests/fixtures/<detector>`, or its
/// `negative/` directory.
fn pair_of(detector: &str, ext: &str, negative: bool) -> (String, String) {
    let mut dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(detector);
    if negative {
        dir.push("negative");
    }
    (
        fs::read_to_string(dir.join(format!("before.{ext}"))).unwrap(),
        fs::read_to_string(dir.join(format!("after.{ext}"))).unwrap(),
    )
}

fn fires(detector: &str, path: &str, needle: &str) {
    let ext = Path::new(path).extension().unwrap().to_str().unwrap();
    let (before, after) = pair_of(detector, ext, false);
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

    let (before, after) = pair_of(detector, ext, true);
    let found = analyze(&before, &after, Path::new(path)).unwrap();
    assert!(
        found.is_empty(),
        "{detector} negative must not fire: {found:?}"
    );
}

#[test]
fn decorator_swap_fires_only_when_the_decorator_name_changes() {
    fires(
        "typescript/decorator-swap",
        "src/users/user.entity.ts",
        "AutoEntity",
    );
}

#[test]
fn import_rewrite_fires_only_when_relative_becomes_alias() {
    fires("typescript/import-rewrite", "src/app.ts", "path aliases");
}

#[test]
fn logger_replacement_fires_only_when_console_becomes_a_logger() {
    fires("typescript/logger-replacement", "src/app.ts", "logger");
}

#[test]
fn type_swap_fires_only_when_interface_becomes_type() {
    fires("typescript/type-swap", "src/app.ts", "type");
}

#[test]
fn function_syntax_fires_only_when_a_declaration_becomes_an_arrow() {
    fires("typescript/function-syntax", "src/app.ts", "arrow");
}

#[test]
fn error_class_fires_only_when_bare_error_becomes_a_custom_class() {
    fires("typescript/error-class", "src/app.ts", "custom error");
}

#[test]
fn control_flow_fires_only_when_else_becomes_a_guard() {
    fires("typescript/control-flow", "src/app.ts", "guard");
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

const THROW_BEFORE: &str =
    "export function f(id: string) {\n  if (!id) throw new Error(\"missing\");\n}\n";
const THROW_AFTER: &str =
    "export function f(id: string) {\n  if (!id) throw new NotFoundError(\"missing\");\n}\n";

fn scopes_in(root: &Path, file: &str) -> Vec<String> {
    let paths = taste_core::config::Paths::for_root(root);
    taste_core::analyze_in(&paths, THROW_BEFORE, THROW_AFTER, Path::new(file))
        .unwrap()
        .into_iter()
        .map(|observation| observation.scope)
        .collect()
}

#[test]
fn a_correction_in_a_monorepo_package_is_scoped_to_that_package() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("apps/api/src/users")).unwrap();
    fs::write(dir.path().join("package.json"), "{}").unwrap();
    fs::write(dir.path().join("apps/api/package.json"), "{}").unwrap();

    assert_eq!(
        scopes_in(dir.path(), "apps/api/src/users/user.service.ts"),
        vec!["apps/api/**/*.service.ts".to_string()]
    );
}

#[test]
fn a_correction_outside_any_nested_package_keeps_its_usual_scope() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/users")).unwrap();
    fs::write(dir.path().join("package.json"), "{}").unwrap();

    assert_eq!(
        scopes_in(dir.path(), "src/users/user.service.ts"),
        vec!["src/**/*.service.ts".to_string()]
    );
    // A folder named like a package but without a manifest is not one.
    fs::create_dir_all(dir.path().join("apps/api/src")).unwrap();
    assert_eq!(
        scopes_in(dir.path(), "apps/api/src/user.service.ts"),
        vec!["apps/**/*.service.ts".to_string()]
    );
}

#[test]
fn any_package_manifest_marks_a_package() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("services/billing/web")).unwrap();
    fs::write(
        dir.path().join("services/billing/go.mod"),
        "module billing\n",
    )
    .unwrap();

    assert_eq!(
        scopes_in(dir.path(), "services/billing/web/user.service.ts"),
        vec!["services/billing/**/*.service.ts".to_string()]
    );
}

#[test]
fn python_relative_import_becoming_first_party_absolute_is_detected() {
    fires(
        "python/import-rewrite",
        "src/myapp/users/views.py",
        "absolute imports",
    );
}

#[test]
fn python_bare_exception_becoming_a_project_exception_is_detected() {
    fires(
        "python/exception-class",
        "src/myapp/users/service.py",
        "exception classes",
    );
}

#[test]
fn python_print_becoming_a_logger_call_is_detected() {
    fires("python/print-to-logger", "src/myapp/sync.py", "`logging`");
}

#[test]
fn python_else_becoming_a_guard_clause_is_detected() {
    fires(
        "python/guard-clause",
        "src/myapp/orders.py",
        "guard clauses",
    );
}

#[test]
fn a_python_import_moved_outside_the_files_own_package_is_not_a_taste() {
    // `other` is not a directory this file lives in, so it is a package.
    let found = analyze(
        "from .models import User\n",
        "from other.models import User\n",
        Path::new("src/myapp/users/views.py"),
    )
    .unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_python_scope_is_derived_like_any_other() {
    let (before, after) = pair_of("python/guard-clause", "py", false);
    let found = analyze(&before, &after, Path::new("app/tests/test_orders.py")).unwrap();
    assert_eq!(found[0].scope, "app/**/test_*.py");
}

#[test]
fn rust_super_path_becoming_a_crate_path_is_detected() {
    fires(
        "rust/use-path-rewrite",
        "src/users/service.rs",
        "`crate::` paths",
    );
}

#[test]
fn rust_unwrap_becoming_a_question_mark_is_detected() {
    fires("rust/unwrap-to-question", "src/config.rs", "`?`");
}

#[test]
fn rust_println_becoming_a_log_macro_is_detected() {
    fires("rust/print-to-log", "src/sync.rs", "`tracing`");
}

#[test]
fn rust_else_becoming_let_else_is_detected() {
    fires("rust/guard-clause", "src/orders.rs", "`let … else`");
}

#[test]
fn rust_corrections_in_tests_main_or_a_binary_are_not_learned() {
    let (before, after) = pair_of("rust/unwrap-to-question", "rs", false);
    for path in ["src/main.rs", "src/bin/tool.rs"] {
        let found = analyze(&before, &after, Path::new(path)).unwrap();
        assert!(found.is_empty(), "{path}: {found:?}");
    }
    let wrap = |body: &str| format!("#[cfg(test)]\nmod tests {{\n{body}}}\n");
    let found = analyze(&wrap(&before), &wrap(&after), Path::new("src/config.rs")).unwrap();
    assert!(found.is_empty(), "{found:?}");
    let in_main = |body: &str| body.replace("pub fn load", "fn main");
    let found = analyze(
        &in_main(&before),
        &in_main(&after),
        Path::new("src/config.rs"),
    )
    .unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn go_bare_error_becoming_a_wrapped_one_is_detected() {
    fires("go/bare-to-wrapped", "internal/config/config.go", "%w");
}

#[test]
fn go_println_becoming_slog_is_detected() {
    fires(
        "go/print-to-structured",
        "internal/sync/run.go",
        "structured logger",
    );
}

#[test]
fn go_else_becoming_an_early_return_is_detected() {
    fires(
        "go/guard-clause",
        "internal/orders/price.go",
        "early returns",
    );
}

#[test]
fn a_go_print_replaced_under_cmd_is_not_learned() {
    let (before, after) = pair_of("go/print-to-structured", "go", false);
    let found = analyze(&before, &after, Path::new("cmd/sync/run.go")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_go_test_file_is_scoped_to_test_files() {
    let (before, after) = pair_of("go/guard-clause", "go", false);
    let found = analyze(&before, &after, Path::new("internal/orders/price_test.go")).unwrap();
    assert_eq!(found[0].scope, "internal/**/*_test.go");
}

/// The non-null-assertion fixture, learned in a project whose tsconfig says `config`.
fn non_null_in(config: Option<&str>, file: &str, negative: bool) -> Vec<taste_core::Observation> {
    let dir = tempfile::tempdir().unwrap();
    if let Some(config) = config {
        fs::write(dir.path().join("tsconfig.json"), config).unwrap();
    }
    let paths = taste_core::config::Paths::for_root(dir.path());
    let (before, after) = pair_of("typescript/non-null-assertion", "ts", negative);
    taste_core::analyze_in(&paths, &before, &after, Path::new(file)).unwrap()
}

const STRICT: &str = "{ \"compilerOptions\": { \"strict\": true } }";

#[test]
fn a_non_null_assertion_replaced_by_optional_chaining_is_learned_under_strict_null_checks() {
    let found = non_null_in(Some(STRICT), "src/greet.ts", false);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].instruction.contains("strictNullChecks"));
    assert_eq!(found[0].anti_pattern.as_deref(), Some("user!.name"));
    assert!(non_null_in(Some(STRICT), "src/greet.ts", true).is_empty());
}

#[test]
fn a_non_null_assertion_removed_without_strict_null_checks_is_not_a_taste() {
    for config in [None, Some("{\"compilerOptions\":{\"strict\":false}}")] {
        assert!(
            non_null_in(config, "src/greet.ts", false).is_empty(),
            "{config:?}"
        );
    }
    // Without a project to read, analyze() can never tell, so it never reports one.
    let (before, after) = pair_of("typescript/non-null-assertion", "ts", false);
    assert!(analyze(&before, &after, Path::new("src/greet.ts"))
        .unwrap()
        .is_empty());
}

#[test]
fn a_non_null_assertion_replaced_by_an_explicit_check_is_learned() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("tsconfig.json"), STRICT).unwrap();
    let paths = taste_core::config::Paths::for_root(dir.path());
    let before = "export function greet(user?: User): string {\n  return user!.name;\n}\n";
    let after = "export function greet(user?: User): string {\n  if (!user) {\n    return \"guest\";\n  }\n  return user.name;\n}\n";
    let found = taste_core::analyze_in(&paths, before, after, Path::new("src/greet.ts")).unwrap();
    assert!(
        found
            .iter()
            .any(|o| o.instruction.contains("strictNullChecks")),
        "{found:?}"
    );
}

#[test]
fn a_yup_schema_rewritten_with_zod_is_detected() {
    fires(
        "typescript/schema-library-swap",
        "src/users/schema.ts",
        "zod",
    );
}

#[test]
fn a_schema_moved_to_zod_while_yup_stays_in_the_file_is_not_a_migration() {
    let before = "import * as yup from 'yup';\nexport const A = yup.object({});\nexport const B = yup.object({});\n";
    let after = "import * as yup from 'yup';\nimport { z } from 'zod';\nexport const A = z.object({});\nexport const B = yup.object({});\n";
    let found = analyze(before, after, Path::new("src/schema.ts")).unwrap();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_service_built_with_new_replaced_by_injection_is_detected() {
    fires(
        "typescript/new-to-injected",
        "src/users/users.service.ts",
        "Inject services",
    );
}

#[test]
fn an_angular_service_moved_to_inject_is_detected() {
    let before = "import { Component } from '@angular/core';\nimport { CartService } from '../cart.service';\n@Component({ selector: 'app-cart' })\nexport class CartComponent {\n  private cart = new CartService();\n}\n";
    let after = "import { Component, inject } from '@angular/core';\nimport { CartService } from '../cart.service';\n@Component({ selector: 'app-cart' })\nexport class CartComponent {\n  private cart = inject(CartService);\n}\n";
    let found = analyze(before, after, Path::new("src/app/cart.component.ts")).unwrap();
    assert!(
        found
            .iter()
            .any(|o| o.instruction.contains("Inject services")),
        "{found:?}"
    );
}
