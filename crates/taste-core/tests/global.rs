//! Personal rules across projects, through `RuleStore` and `GlobalStore`.
//!
//! Each test gets its own home directory, so nothing here touches the real
//! `~/.taste`.

mod common;

use common::Project;
use taste_core::ast::{PREFER_ARROW_FUNCTIONS, PREFER_FUNCTION_DECLARATIONS};
use taste_core::config::Paths;
use taste_core::models::{RuleCategory, RuleSource};
use taste_core::storage::{GlobalStore, Observation, RuleStore};

/// A project whose store is attached to the personal rules in `home`.
fn project_in(home: &std::path::Path) -> (tempfile::TempDir, RuleStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = RuleStore::open(Paths::for_root(dir.path()))
        .unwrap()
        .with_global(GlobalStore::open(home).unwrap());
    (dir, store)
}

fn confirm(store: &RuleStore, scope: &str, instruction: &str) -> String {
    store
        .record_observation(Observation::new(
            RuleCategory::CodeStyle,
            scope,
            RuleSource::Manual,
            instruction,
        ))
        .unwrap()
        .promoted()
        .unwrap()
        .id
        .clone()
}

#[test]
fn a_rule_confirmed_in_two_projects_is_suggested_as_personal() {
    let home = tempfile::tempdir().unwrap();
    let (_a, first) = project_in(home.path());
    let (_b, second) = project_in(home.path());
    confirm(&first, "**/*.ts", PREFER_FUNCTION_DECLARATIONS);
    assert!(GlobalStore::open(home.path())
        .unwrap()
        .suggestions()
        .unwrap()
        .is_empty());
    confirm(&second, "**/*.ts", PREFER_FUNCTION_DECLARATIONS);
    let suggestions = GlobalStore::open(home.path())
        .unwrap()
        .suggestions()
        .unwrap();
    assert_eq!(suggestions.len(), 1);
    assert_eq!(
        suggestions[0].rule.instruction,
        PREFER_FUNCTION_DECLARATIONS
    );
    assert_eq!(suggestions[0].projects.len(), 2);
}

#[test]
fn a_personal_rule_is_served_where_its_scope_matches_and_a_project_rule_wins() {
    let home = tempfile::tempdir().unwrap();
    let (_a, origin) = project_in(home.path());
    let id = confirm(&origin, "**/*.ts", PREFER_FUNCTION_DECLARATIONS);
    let rule = origin.rules().unwrap().find(&id).unwrap().clone();
    origin
        .global()
        .unwrap()
        .add(&rule, false, &origin.paths().project_root)
        .unwrap();

    let (b, elsewhere) = project_in(home.path());
    let served = elsewhere
        .personal_rules_for_file(b.path().join("src/app.ts"))
        .unwrap();
    assert_eq!(served.len(), 1);
    assert!(elsewhere
        .personal_rules_for_file(b.path().join("src/app.py"))
        .unwrap()
        .is_empty());
    assert!(
        origin
            .personal_rules_for_file("src/app.ts")
            .unwrap()
            .is_empty(),
        "the project that has the rule already serves it as its own"
    );

    // A project that confirmed the opposite does not get it.
    let (c, opposite) = project_in(home.path());
    confirm(&opposite, "**/*.ts", PREFER_ARROW_FUNCTIONS);
    assert!(opposite
        .personal_rules_for_file(c.path().join("src/app.ts"))
        .unwrap()
        .is_empty());
}

#[test]
fn a_project_that_opts_out_is_served_no_personal_rules() {
    let home = tempfile::tempdir().unwrap();
    let (_a, origin) = project_in(home.path());
    let id = confirm(&origin, "**/*", "Write commit messages in the imperative.");
    let rule = origin.rules().unwrap().find(&id).unwrap().clone();
    origin
        .global()
        .unwrap()
        .add(&rule, false, &origin.paths().project_root)
        .unwrap();

    let (_b, other) = project_in(home.path());
    assert_eq!(other.personal_rules_for_file("src/a.ts").unwrap().len(), 1);
    other.set_global_off(true).unwrap();
    assert!(other
        .personal_rules_for_file("src/a.ts")
        .unwrap()
        .is_empty());
    other.set_global_off(false).unwrap();
    assert_eq!(other.personal_rules_for_file("src/a.ts").unwrap().len(), 1);
}

#[test]
fn a_store_without_personal_rules_attached_serves_none() {
    let project = Project::with_cache_db();
    assert!(project
        .store
        .personal_rules_for_file("src/a.ts")
        .unwrap()
        .is_empty());
    assert!(project.store.global().is_none());
}
