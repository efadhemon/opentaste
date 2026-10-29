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
