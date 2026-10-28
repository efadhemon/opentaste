//! What the generated files actually look like.
//!
//! The snapshots here are the readable record of the contract with each
//! editor: change the renderer and the diff shows exactly what every Cursor,
//! Claude, Windsurf and Copilot user would start seeing.

mod common;

use common::{rule, Project};
use taste_core::models::{RuleCategory, TasteRule};
use taste_core::transpiler::{cursor_mdc, marker, render};

/// A rule set covering the interesting rendering cases: multiple categories,
/// several scopes in one category, a project-wide scope, and both inline and
/// multi-line example snippets.
fn rules() -> Vec<TasteRule> {
    vec![
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
        TasteRule {
            anti_pattern: Some("return this.userService.findAll();".to_string()),
            preferred_pattern: Some(
                "return ApiResponse.ok(await this.userService.findAll());".to_string(),
            ),
            ..rule(
                "arch-standard-api-response",
                RuleCategory::Architecture,
                "src/**/*.controller.ts",
                "Always return a standardized ApiResponse<T> wrapper from controller methods.",
            )
        },
        rule(
            "style-early-returns",
            RuleCategory::CodeStyle,
            "**/*",
            "Prefer early returns (guard clauses) over nested if-else blocks.",
        ),
        TasteRule {
            anti_pattern: Some("import { User } from '../../users/user.entity';".to_string()),
            preferred_pattern: Some("import { User } from '@/users/user.entity';".to_string()),
            ..rule(
                "imports-alias-paths",
                RuleCategory::Imports,
                "src/**/*.ts",
                "Import through the @/ path alias rather than deep relative paths.",
            )
        },
    ]
}

fn confirmed(project: &Project) -> Vec<TasteRule> {
    rules().iter().map(|r| project.confirm_like(r)).collect()
}

#[test]
fn the_shared_marker_body_reads_the_way_the_spec_describes() {
    insta::assert_snapshot!("marker_body", render::sections(&rules()));
}

#[test]
fn a_scoped_cursor_rule_file_carries_valid_frontmatter() {
    let all = rules();
    let architecture: Vec<&TasteRule> = all
        .iter()
        .filter(|r| r.category == RuleCategory::Architecture)
        .collect();

    insta::assert_snapshot!(
        "cursor_architecture_mdc",
        cursor_mdc::render_file(RuleCategory::Architecture, &architecture)
    );
}

#[test]
fn a_project_wide_cursor_rule_file_always_applies() {
    let all = rules();
    let style: Vec<&TasteRule> = all
        .iter()
        .filter(|r| r.category == RuleCategory::CodeStyle)
        .collect();

    insta::assert_snapshot!(
        "cursor_code_style_mdc",
        cursor_mdc::render_file(RuleCategory::CodeStyle, &style)
    );
}

#[test]
fn claude_md_is_created_from_scratch_when_the_project_has_none() {
    let project = Project::new();
    confirmed(&project);
    project.store.sync().unwrap();

    insta::assert_snapshot!("claude_md", project.read("CLAUDE.md").unwrap());
}

#[test]
fn a_full_sync_writes_exactly_the_documented_set_of_files() {
    let project = Project::new();
    confirmed(&project);
    let report = project.store.sync().unwrap();

    let mut written: Vec<String> = report
        .written
        .iter()
        .map(|p| {
            project
                .paths
                .relativize(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    written.sort();

    let mut expected = vec![
        ".cursor/rules/opentaste-architecture.mdc",
        ".cursor/rules/opentaste-code-style.mdc",
        ".cursor/rules/opentaste-imports.mdc",
        ".github/copilot-instructions.md",
        ".windsurfrules",
        "AGENTS.md",
        "CLAUDE.md",
    ];
    expected.sort();

    assert_eq!(written, expected);
}

#[test]
fn every_marker_target_receives_the_same_body() {
    let project = Project::new();
    confirmed(&project);
    project.store.sync().unwrap();

    let bodies: Vec<String> = [
        "CLAUDE.md",
        ".windsurfrules",
        "AGENTS.md",
        ".github/copilot-instructions.md",
    ]
    .iter()
    .map(|t| project.read(t).expect(t))
    .collect();

    for body in &bodies[1..] {
        assert_eq!(
            body, &bodies[0],
            "all marker targets should receive identical content"
        );
    }
    assert!(marker::has_block(&bodies[0]));
}

#[test]
fn a_cursor_file_is_written_per_category_and_no_others() {
    let project = Project::new();
    confirmed(&project);
    project.store.sync().unwrap();

    let dir = project.paths.cursor_rules_dir();
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();

    assert_eq!(
        names,
        vec![
            "opentaste-architecture.mdc",
            "opentaste-code-style.mdc",
            "opentaste-imports.mdc",
        ]
    );
}

#[test]
fn deleting_a_categorys_last_rule_deletes_its_cursor_file() {
    let project = Project::new();
    let stored = confirmed(&project);
    project.store.sync().unwrap();
    assert!(project.exists(".cursor/rules/opentaste-imports.mdc"));

    let imports = stored
        .iter()
        .find(|r| r.category == RuleCategory::Imports)
        .unwrap();
    project.store.delete_rule(&imports.id).unwrap();
    let report = project.store.sync().unwrap();

    assert!(!project.exists(".cursor/rules/opentaste-imports.mdc"));
    assert!(
        report
            .removed
            .contains(&cursor_mdc::file_for(&project.paths, RuleCategory::Imports)),
        "{report:#?}"
    );
    assert!(
        project.exists(".cursor/rules/opentaste-architecture.mdc"),
        "other categories should be untouched"
    );
}
