//! The git-tracked rule file has to behave itself in a repository: the same
//! rule set must always produce the same bytes, and a hand-edited file must
//! load without surprises.

mod common;

use common::{fixed_time, rule, Project};
use pretty_assertions::assert_eq;
use taste_core::models::{ProjectContext, RuleCategory, RuleSource, RulesFile, TasteRule};
use taste_core::storage::JsonStore;

fn sample_rules() -> RulesFile {
    RulesFile {
        project_context: ProjectContext {
            detected_stack: vec![
                "typescript".to_string(),
                "nestjs".to_string(),
                "react".to_string(),
            ],
            total_files_scanned: 342,
            last_updated: fixed_time(),
        },
        rules: vec![
            TasteRule {
                anti_pattern: Some("@Entity('users')\nexport class User {}".to_string()),
                preferred_pattern: Some("@AutoEntity()\nexport class User {}".to_string()),
                ..rule(
                    "arch-custom-entity-decorators",
                    RuleCategory::Architecture,
                    "src/**/*.entity.ts",
                    "Use custom @AutoEntity() and @AutoIndex() decorators instead of raw TypeORM @Entity() and @Index().",
                )
            },
            rule(
                "style-early-returns",
                RuleCategory::CodeStyle,
                "**/*",
                "Prefer early returns (guard clauses) over nested if-else blocks.",
            ),
        ],
        ..Default::default()
    }
}

#[test]
fn the_rule_file_matches_the_shape_documented_in_the_spec() {
    let project = Project::new();
    let store = JsonStore::new(&project.paths.rules_json);
    store.save(&sample_rules()).unwrap();

    let raw = project
        .read(".taste/rules.json")
        .expect("rules.json written");
    insta::assert_snapshot!("rules_json", raw);
}

#[test]
fn an_unchanged_rule_set_reproduces_the_file_byte_for_byte() {
    let project = Project::new();
    let store = JsonStore::new(&project.paths.rules_json);

    assert!(store.save(&sample_rules()).unwrap());
    let first = std::fs::read(&project.paths.rules_json).unwrap();

    // Re-saving, and saving a round-tripped copy, must both be no-ops.
    assert!(!store.save(&sample_rules()).unwrap());
    assert!(!store.save(&store.load().unwrap()).unwrap());

    assert_eq!(std::fs::read(&project.paths.rules_json).unwrap(), first);
}

#[test]
fn rule_order_on_disk_does_not_depend_on_insertion_order() {
    let project = Project::new();
    let store = JsonStore::new(&project.paths.rules_json);

    let forwards = sample_rules();
    let mut backwards = sample_rules();
    backwards.rules.reverse();

    store.save(&forwards).unwrap();
    let a = std::fs::read(&project.paths.rules_json).unwrap();

    std::fs::remove_file(&project.paths.rules_json).unwrap();
    store.save(&backwards).unwrap();
    let b = std::fs::read(&project.paths.rules_json).unwrap();

    assert_eq!(a, b, "insertion order must not leak into the file");
}

#[test]
fn a_hand_written_rule_file_loads_and_normalises() {
    let project = Project::new();
    project.write(
        ".taste/rules.json",
        r#"{
  "$schema": "https://raw.githubusercontent.com/efadhemon/opentaste/main/schema/v1.json",
  "version": "1.0.0",
  "project_context": {
    "detected_stack": ["typescript"],
    "total_files_scanned": 12,
    "last_updated": "2026-10-29T00:00:00Z"
  },
  "rules": [
    {
      "id": "z-written-last",
      "category": "error-handling",
      "scope": "src/**/*.ts",
      "confidence": 0.8500000238418579,
      "occurrences": 5,
      "enabled": true,
      "source": "mcp-feedback",
      "instruction": "Throw AppException rather than the built-in Error.",
      "updated_at": "2026-10-29T00:00:00Z"
    },
    {
      "id": "a-written-first",
      "category": "naming",
      "scope": "**/*",
      "confidence": 0.9,
      "occurrences": 3,
      "enabled": false,
      "source": "manual",
      "instruction": "Name boolean fields with an is or has prefix.",
      "updated_at": "2026-10-29T00:00:00Z"
    }
  ]
}
"#,
    );

    let store = JsonStore::new(&project.paths.rules_json);
    let loaded = store.load().unwrap();

    assert_eq!(
        loaded
            .rules
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-written-first", "z-written-last"],
        "loading must impose canonical order"
    );
    assert_eq!(loaded.rules[1].category, RuleCategory::ErrorHandling);
    assert_eq!(loaded.rules[1].source, RuleSource::McpFeedback);
    assert!(!loaded.rules[0].enabled);

    // The float noise a hand edit (or another tool) introduced is normalised
    // away, and saving tidies the file rather than preserving the noise.
    assert_eq!(loaded.rules[1].confidence, 0.85);
    assert!(store.save(&loaded).unwrap(), "the file should be rewritten");
    let rewritten = project.read(".taste/rules.json").unwrap();
    assert!(rewritten.contains(r#""confidence": 0.85,"#), "{rewritten}");
    assert!(!rewritten.contains("0.8500000238"), "{rewritten}");
}

#[test]
fn optional_pattern_fields_are_omitted_rather_than_written_as_null() {
    let project = Project::new();
    let store = JsonStore::new(&project.paths.rules_json);

    store
        .save(&RulesFile::with_rules([rule(
            "naming-booleans",
            RuleCategory::Naming,
            "**/*",
            "Name boolean fields with an is or has prefix.",
        )]))
        .unwrap();

    let raw = project.read(".taste/rules.json").unwrap();
    assert!(!raw.contains("null"), "{raw}");
    assert!(!raw.contains("anti_pattern"), "{raw}");

    // And they still round-trip as absent.
    assert!(store.load().unwrap().rules[0].anti_pattern.is_none());
}
