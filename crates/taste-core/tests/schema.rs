//! `schema/v1.json` is what editors validate `.taste/rules.json` against, from
//! the URL every rule file carries. It is written by hand, so these tests keep
//! it in step with what the Rust model actually serialises.

mod common;

use std::collections::BTreeSet;

use common::{rule, Project};
use pretty_assertions::assert_eq;
use serde_json::Value;
use taste_core::config::SCHEMA_URL;
use taste_core::models::{RuleCategory, RuleSource, RulesFile, TasteRule};

fn schema() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema/v1.json");
    let raw = std::fs::read_to_string(path).expect("schema/v1.json exists");
    serde_json::from_str(&raw).expect("schema/v1.json is valid JSON")
}

fn strings(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|v| v.as_str().expect("a string").to_string())
        .collect()
}

fn keys(value: &Value) -> BTreeSet<String> {
    value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect()
}

fn full_rule() -> TasteRule {
    TasteRule {
        anti_pattern: Some("before".to_string()),
        preferred_pattern: Some("after".to_string()),
        ..rule("style-x", RuleCategory::CodeStyle, "**/*", "Do x.")
    }
}

#[test]
fn the_schema_is_served_from_the_url_every_rule_file_carries() {
    assert_eq!(schema()["$id"], SCHEMA_URL);
}

#[test]
fn the_schema_lists_exactly_the_top_level_fields_a_rule_file_has() {
    let schema = schema();
    let written = keys(&serde_json::to_value(RulesFile::default()).unwrap());
    assert_eq!(keys(&schema["properties"]), written);
    assert_eq!(strings(&schema["required"]), written);

    let context = &schema["properties"]["project_context"];
    let written = keys(&serde_json::to_value(RulesFile::default().project_context).unwrap());
    assert_eq!(keys(&context["properties"]), written);
    assert_eq!(strings(&context["required"]), written);
}

#[test]
fn the_schema_requires_every_rule_field_except_the_optional_patterns() {
    let rule_schema = &schema()["$defs"]["rule"];
    let all = keys(&serde_json::to_value(full_rule()).unwrap());
    let required = keys(
        &serde_json::to_value(TasteRule {
            anti_pattern: None,
            preferred_pattern: None,
            ..full_rule()
        })
        .unwrap(),
    );
    assert_eq!(keys(&rule_schema["properties"]), all);
    assert_eq!(strings(&rule_schema["required"]), required);
}

#[test]
fn the_schema_enums_match_every_category_and_source() {
    let rule_schema = &schema()["$defs"]["rule"];
    let categories: BTreeSet<String> = RuleCategory::ALL
        .iter()
        .map(|c| c.slug().to_string())
        .collect();
    assert_eq!(
        strings(&rule_schema["properties"]["category"]["enum"]),
        categories
    );

    let sources: BTreeSet<String> = [
        RuleSource::AstScan,
        RuleSource::AstDiff,
        RuleSource::McpFeedback,
        RuleSource::Manual,
    ]
    .iter()
    .map(|s| s.slug().to_string())
    .collect();
    assert_eq!(
        strings(&rule_schema["properties"]["source"]["enum"]),
        sources
    );
}

#[test]
fn a_rule_file_written_by_the_store_names_the_schema_url() {
    let project = Project::new();
    project.confirm(RuleCategory::CodeStyle, "**/*", "Prefer early returns.");
    let raw = project
        .read(".taste/rules.json")
        .expect("rules.json written");
    let written: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(written["$schema"], SCHEMA_URL);
}
