//! End-to-end exercise of the learning loop against real files on disk.
//!
//! These tests use a real `.taste/cache.db` rather than an in-memory one, so
//! they also cover the part that matters most: a rule only ever living in one
//! of the two stores, never in both and never in neither.

mod common;

use common::Project;
use taste_core::config::{DEMOTION_CONFIDENCE, PROMOTION_CONFIDENCE};
use taste_core::models::{RuleCategory, RuleSource};
use taste_core::storage::{Observation, Outcome};

fn entity_decorator() -> Observation {
    Observation::new(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        RuleSource::AstDiff,
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    )
    .with_patterns(
        Some("@Entity('users')".to_string()),
        Some("@AutoEntity()".to_string()),
    )
}

#[test]
fn a_repeatedly_corrected_pattern_graduates_into_the_git_tracked_file() {
    let project = Project::with_cache_db();

    let mut confidences = Vec::new();
    let mut promoted_on = None;

    for attempt in 1..=6 {
        let outcome = project
            .store
            .record_observation(entity_decorator())
            .unwrap();

        match &outcome {
            Outcome::Created { confidence, .. } | Outcome::Reinforced { confidence, .. } => {
                confidences.push(*confidence);
                assert!(
                    project.store.rules().unwrap().rules.is_empty(),
                    "an unpromoted rule must stay out of git"
                );
                assert_eq!(project.store.candidate_rules().unwrap().len(), 1);
            }
            Outcome::Promoted(rule) => {
                confidences.push(rule.confidence);
                promoted_on = Some(attempt);
                break;
            }
            Outcome::AlreadyConfirmed { .. } => panic!("promotion should come first"),
        }
    }

    assert_eq!(promoted_on, Some(6), "confidences so far: {confidences:?}");
    assert_eq!(confidences, vec![0.45, 0.56, 0.65, 0.72, 0.78, 0.82]);

    // Exactly one home: present in the rule file, absent from the cache.
    let rules = project.store.rules().unwrap();
    assert_eq!(rules.rules.len(), 1);
    assert!(project.store.candidate_rules().unwrap().is_empty());

    let rule = &rules.rules[0];
    assert!(rule.confidence >= PROMOTION_CONFIDENCE);
    assert_eq!(rule.occurrences, 6);
    assert_eq!(rule.source, RuleSource::AstDiff);
    assert_eq!(rule.anti_pattern.as_deref(), Some("@Entity('users')"));
    assert_eq!(rule.preferred_pattern.as_deref(), Some("@AutoEntity()"));
    assert!(project.exists(".taste/rules.json"));
    assert!(project.exists(".taste/cache.db"));
}

#[test]
fn an_explicit_chat_correction_is_trusted_at_once() {
    let project = Project::with_cache_db();

    let rule = project
        .store
        .record_observation(
            Observation::new(
                RuleCategory::Architecture,
                "src/**/*.controller.ts",
                RuleSource::McpFeedback,
                "Always return standardized ApiResponse<T> wrapper from controller methods",
            )
            .with_patterns(
                Some("return this.userService.findAll();".to_string()),
                Some("return ApiResponse.ok(await this.userService.findAll());".to_string()),
            ),
        )
        .unwrap()
        .promoted()
        .expect("explicit feedback promotes on the first sighting")
        .clone();

    assert_eq!(rule.confidence, 0.95);
    assert_eq!(rule.occurrences, 1);
    assert!(rule.enabled);
    assert!(
        project.store.candidate_rules().unwrap().is_empty(),
        "an immediately promoted rule should never touch the candidate cache"
    );
    assert_eq!(project.store.rules().unwrap().rules.len(), 1);
}

#[test]
fn contradicting_a_rule_demotes_it_back_to_a_candidate_and_only_there() {
    let project = Project::with_cache_db();
    let rule = project.confirm(
        RuleCategory::ErrorHandling,
        "src/**/*.ts",
        "Throw AppException rather than the built-in Error",
    );

    // 0.95 -> 0.71 -> 0.53: confirmed but inactive, so the editor stops hearing
    // about it well before the rule is actually given up on.
    project.store.apply_contradiction(&rule.id).unwrap();
    project.store.apply_contradiction(&rule.id).unwrap();

    let still_confirmed = project.store.rules().unwrap();
    assert_eq!(still_confirmed.find(&rule.id).unwrap().confidence, 0.53);
    assert!(still_confirmed.find(&rule.id).unwrap().confidence >= DEMOTION_CONFIDENCE);
    assert!(project.store.active_rules().unwrap().is_empty());
    assert!(project.store.candidate_rules().unwrap().is_empty());

    // 0.40 crosses the line and the rule leaves the repository entirely.
    project.store.apply_contradiction(&rule.id).unwrap();

    assert!(project.store.rules().unwrap().find(&rule.id).is_none());
    let candidates = project.store.candidate_rules().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, rule.id);
    assert_eq!(candidates[0].contradictions, 1);
    assert_eq!(
        candidates[0].instruction, rule.instruction,
        "a demoted rule keeps its wording so it can be relearned"
    );
}

#[test]
fn a_demoted_rule_can_earn_its_way_back() {
    let project = Project::with_cache_db();
    let rule = project.confirm(
        RuleCategory::ErrorHandling,
        "src/**/*.ts",
        "Throw AppException rather than the built-in Error",
    );

    for _ in 0..3 {
        project.store.apply_contradiction(&rule.id).unwrap();
    }
    assert!(project.store.rules().unwrap().find(&rule.id).is_none());

    // Observing it again reinforces the surviving candidate rather than
    // starting from scratch, and explicit confirmation restores it.
    let restored = project
        .store
        .record_observation(Observation::new(
            RuleCategory::ErrorHandling,
            "src/**/*.ts",
            RuleSource::McpFeedback,
            "Throw AppException rather than the built-in Error",
        ))
        .unwrap()
        .promoted()
        .expect("explicit confirmation restores a demoted rule")
        .clone();

    assert_eq!(restored.id, rule.id);
    assert_eq!(restored.occurrences, 2);
    assert!(project.store.candidate_rules().unwrap().is_empty());
}

#[test]
fn independent_preferences_accumulate_side_by_side() {
    let project = Project::with_cache_db();

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.confirm(
        RuleCategory::CodeStyle,
        "**/*",
        "Prefer early returns (guard clauses) over nested if-else blocks",
    );
    project.confirm(
        RuleCategory::Imports,
        "src/**/*.ts",
        "Import from the @/ alias rather than deep relative paths",
    );

    let rules = project.store.rules().unwrap();
    assert_eq!(rules.rules.len(), 3);

    let mut ids: Vec<_> = rules.rules.iter().map(|r| r.id.clone()).collect();
    let sorted = {
        let mut s = ids.clone();
        s.sort();
        s
    };
    assert_eq!(ids, sorted, "the file must stay sorted by id");

    ids.dedup();
    assert_eq!(ids.len(), 3, "identifiers must be distinct");
}

#[test]
fn the_store_survives_being_reopened_between_observations() {
    let dir = tempfile::tempdir().unwrap();
    let paths = taste_core::config::Paths::for_root(dir.path());
    let instruction = "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()";

    // Six separate processes, each opening the store fresh, as a git hook
    // firing once per commit would.
    for _ in 0..6 {
        let store = taste_core::storage::RuleStore::open(paths.clone()).unwrap();
        store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                RuleSource::AstDiff,
                instruction,
            ))
            .unwrap();
    }

    let store = taste_core::storage::RuleStore::open(paths).unwrap();
    let rules = store.rules().unwrap();
    assert_eq!(rules.rules.len(), 1, "{rules:#?}");
    assert_eq!(rules.rules[0].occurrences, 6);
    assert!(store.candidate_rules().unwrap().is_empty());
}
