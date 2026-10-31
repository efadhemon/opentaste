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
            Outcome::Conflicts { .. } => panic!("no opposite rule exists"),
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

/// Confirm a rule from chat whose anti-pattern is a bare `Error` throw.
fn confirm_throw_rule(project: &Project) -> String {
    project
        .store
        .record_observation(
            Observation::new(
                RuleCategory::ErrorHandling,
                "src/**/*.ts",
                RuleSource::McpFeedback,
                "Throw AppException rather than the built-in Error",
            )
            .with_patterns(
                Some("throw new Error(\"missing\")".to_string()),
                Some("throw new AppException(\"missing\")".to_string()),
            ),
        )
        .unwrap()
        .promoted()
        .expect("chat feedback is confirmed at once")
        .id
        .clone()
}

/// One edit to `src/app.ts` that adds one more bare `Error` throw.
fn contradict_once(project: &Project, throws_before: usize) -> Vec<String> {
    let body = |n: usize| -> String {
        (0..n)
            .map(|i| format!("export function f{i}() {{\n  throw new Error(\"missing\");\n}}\n"))
            .collect()
    };
    let ids = taste_core::contradictions(
        &project.store,
        std::path::Path::new("src/app.ts"),
        &body(throws_before),
        &body(throws_before + 1),
    )
    .unwrap();
    for id in &ids {
        project.store.apply_contradiction(id).unwrap();
    }
    ids
}

#[test]
fn a_rule_that_keeps_being_contradicted_is_demoted_through_detection() {
    let project = Project::with_cache_db();
    let id = confirm_throw_rule(&project);

    // 0.95 -> 0.71: out of the editor files, still in the rule file.
    assert_eq!(contradict_once(&project, 0), vec![id.clone()]);
    assert!(project.store.active_rules().unwrap().is_empty());
    assert_eq!(
        project.store.rules().unwrap().find(&id).unwrap().confidence,
        0.71
    );

    // An inactive rule is still checked, so the next contradiction lands.
    assert_eq!(contradict_once(&project, 1), vec![id.clone()]);
    assert_eq!(
        project.store.rules().unwrap().find(&id).unwrap().confidence,
        0.53
    );

    // 0.40 crosses the demotion line.
    assert_eq!(contradict_once(&project, 2), vec![id.clone()]);
    assert!(project.store.rules().unwrap().find(&id).is_none());
    let candidates = project.store.candidate_rules().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].id, id);
}

#[test]
fn a_disabled_rule_is_never_contradicted() {
    let project = Project::with_cache_db();
    let id = confirm_throw_rule(&project);
    assert!(project.store.set_enabled(&id, false).unwrap());

    assert!(contradict_once(&project, 0).is_empty());
    let rule = project.store.rules().unwrap().find(&id).unwrap().clone();
    assert_eq!(rule.confidence, 0.95);
    assert!(!rule.enabled);
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
fn a_correction_in_one_package_never_reinforces_another_packages_rule() {
    let project = Project::with_cache_db();
    let instruction = "Prefer custom error classes over throwing bare `Error`.";
    let observe = |scope: &str| {
        project
            .store
            .record_observation(Observation::new(
                RuleCategory::ErrorHandling,
                scope,
                RuleSource::AstDiff,
                instruction,
            ))
            .unwrap()
    };

    assert!(matches!(
        observe("apps/nextjs/**/*.tsx"),
        Outcome::Created { .. }
    ));
    assert!(
        matches!(
            observe("apps/tanstack-start/**/*.tsx"),
            Outcome::Created { .. }
        ),
        "a second package starts its own candidate"
    );

    let mut candidates = project.store.candidate_rules().unwrap();
    candidates.sort_by(|a, b| a.scope.cmp(&b.scope));
    let scopes: Vec<_> = candidates
        .iter()
        .map(|c| (c.scope.as_str(), c.occurrences))
        .collect();
    assert_eq!(
        scopes,
        vec![
            ("apps/nextjs/**/*.tsx", 1),
            ("apps/tanstack-start/**/*.tsx", 1)
        ]
    );
    assert_ne!(candidates[0].id, candidates[1].id);
}

#[test]
fn a_chat_phrasing_of_a_detected_edit_reinforces_the_same_candidate() {
    let project = Project::with_cache_db();
    let observe = |instruction: &str, anti: &str, preferred: &str| {
        project
            .store
            .record_observation(
                Observation::new(
                    RuleCategory::CodeStyle,
                    "src/**/*.ts",
                    RuleSource::AstDiff,
                    instruction,
                )
                .with_patterns(Some(anti.to_string()), Some(preferred.to_string())),
            )
            .unwrap()
    };

    observe(
        "Prefer a logger service over `console.log` and `console.error`.",
        "console.log(user)",
        "logger.info(user)",
    );
    let outcome = observe(
        "Write to the logger, never to the console.",
        "console.log(order)",
        "logger.info(order)",
    );
    assert!(
        matches!(outcome, Outcome::Reinforced { occurrences: 2, .. }),
        "{outcome:?}"
    );

    observe(
        "Write to the logger, never to the console.",
        "console.error(order)",
        "logger.error(order)",
    );
    assert_eq!(
        project.store.candidate_rules().unwrap().len(),
        2,
        "a different call is a different edit, and its wording matches neither"
    );
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

#[test]
fn a_correction_towards_one_side_decays_the_candidate_for_the_other() {
    use taste_core::ast::{PREFER_ARROW_FUNCTIONS, PREFER_FUNCTION_DECLARATIONS};
    let project = Project::with_cache_db();
    let observe = |instruction: &str, scope: &str| {
        project
            .store
            .record_observation(Observation::new(
                RuleCategory::CodeStyle,
                scope,
                RuleSource::AstDiff,
                instruction,
            ))
            .unwrap()
    };
    // The team used arrows, then moved to function declarations.
    observe(PREFER_ARROW_FUNCTIONS, "packages/ui/**/*.tsx");
    observe(PREFER_ARROW_FUNCTIONS, "packages/ui/**/*.tsx");
    observe(PREFER_ARROW_FUNCTIONS, "apps/web/**/*.tsx");
    for _ in 0..3 {
        observe(PREFER_FUNCTION_DECLARATIONS, "packages/ui/**/*.tsx");
    }
    let candidates = project.store.candidate_rules().unwrap();
    let find = |instruction: &str, scope: &str| {
        candidates
            .iter()
            .find(|c| c.instruction == instruction && c.scope == scope)
            .unwrap()
            .clone()
    };
    let arrows = find(PREFER_ARROW_FUNCTIONS, "packages/ui/**/*.tsx");
    let declarations = find(PREFER_FUNCTION_DECLARATIONS, "packages/ui/**/*.tsx");
    assert_eq!(arrows.contradictions, 3);
    assert!(arrows.confidence < 0.30, "{}", arrows.confidence);
    assert_eq!(declarations.confidence, 0.65);
    assert_eq!(
        find(PREFER_ARROW_FUNCTIONS, "apps/web/**/*.tsx").contradictions,
        0,
        "another package's candidate is untouched"
    );
}

mod editing {
    use super::*;
    use taste_core::Edit;

    fn confirmed(project: &Project, scope: &str, instruction: &str) -> String {
        project
            .store
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
    fn a_confirmed_rule_is_reworded_and_rescoped_in_place_keeping_its_id() {
        let project = Project::with_cache_db();
        let id = confirmed(&project, "src/**/*.ts", "Prefer early returns.");
        let outcome = project
            .store
            .update_rule(
                &id,
                "Prefer early returns over nested else.",
                " apps/**/*.ts ",
            )
            .unwrap();
        assert_eq!(outcome, Edit::Updated);
        let rules = project.store.rules().unwrap();
        let rule = rules.find(&id).expect("same id");
        assert_eq!(rule.instruction, "Prefer early returns over nested else.");
        assert_eq!(rule.scope, "apps/**/*.ts");
    }

    #[test]
    fn a_candidate_is_edited_in_the_cache() {
        let project = Project::with_cache_db();
        let created = project
            .store
            .record_observation(Observation::new(
                RuleCategory::CodeStyle,
                "src/**/*.ts",
                RuleSource::AstDiff,
                "Prefer a logger.",
            ))
            .unwrap();
        let Outcome::Created { id, .. } = created else {
            panic!("{created:?}");
        };
        assert_eq!(
            project
                .store
                .update_rule(&id, "Log through the logger service.", "src/**/*.ts")
                .unwrap(),
            Edit::Updated
        );
        let candidate = project.store.candidate_rules().unwrap().pop().unwrap();
        assert_eq!(candidate.id, id);
        assert_eq!(candidate.instruction, "Log through the logger service.");
        assert!(project.store.rules().unwrap().rules.is_empty());
    }

    #[test]
    fn an_invalid_scope_an_empty_instruction_or_a_duplicate_writes_nothing() {
        let project = Project::with_cache_db();
        let id = confirmed(&project, "src/**/*.ts", "Prefer early returns.");
        let other = confirmed(&project, "src/**/*.ts", "Prefer named exports.");
        let before = project.store.rules().unwrap();

        assert!(project
            .store
            .update_rule(&id, "Prefer early returns.", "src/[")
            .is_err());
        assert_eq!(
            project
                .store
                .update_rule(&id, "   ", "src/**/*.ts")
                .unwrap(),
            Edit::EmptyInstruction
        );
        assert_eq!(
            project
                .store
                .update_rule(&id, "Prefer named exports!", "src/**/*.ts")
                .unwrap(),
            Edit::Duplicate(other),
            "rewording one rule into another is not an edit"
        );
        assert_eq!(project.store.rules().unwrap(), before);
        assert_eq!(
            project.store.update_rule("nope", "x", "**/*").unwrap(),
            Edit::NotFound
        );
    }
}
