//! The guarantee that matters most in the whole transpiler: a developer's own
//! words in `CLAUDE.md`, `.windsurfrules`, `AGENTS.md` and the Copilot
//! instructions are never altered, reordered or lost.
//!
//! These files are often long-lived and hand-curated. Getting this wrong once
//! would be enough to make the tool untrustworthy, so every target is checked
//! for byte-level preservation rather than "looks about right".

mod common;

use common::Project;
use pretty_assertions::assert_eq;
use taste_core::models::RuleCategory;
use taste_core::transpiler::{self, marker};
use taste_core::Error;

/// A realistic handwritten file with content on both sides of where our block
/// will land, including prose that mentions markdown our renderer also emits.
const HANDWRITTEN: &str = "\
# Project Conventions

These notes are maintained by the team. Please keep them current.

## Build

Run `pnpm build` before pushing.

## Review checklist

- Does it have tests?
- Does it read well?
";

const TRAILING: &str = "\
## Contact

Ask in #eng-help.
";

fn marker_targets(project: &Project) -> Vec<&'static str> {
    let _ = project;
    vec![
        "CLAUDE.md",
        ".windsurfrules",
        "AGENTS.md",
        ".github/copilot-instructions.md",
    ]
}

#[test]
fn handwritten_content_is_preserved_in_every_marker_target() {
    let project = Project::new();
    for target in marker_targets(&project) {
        project.write(target, HANDWRITTEN);
    }

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.store.sync().unwrap();

    for target in marker_targets(&project) {
        let updated = project.read(target).expect(target);
        assert!(
            updated.starts_with(HANDWRITTEN),
            "{target} lost its handwritten prefix:\n{updated}"
        );
        assert!(updated.contains("@AutoEntity()"), "{target}:\n{updated}");
        assert!(marker::has_block(&updated), "{target}:\n{updated}");
    }
}

#[test]
fn content_on_both_sides_of_the_block_survives_repeated_edits() {
    let project = Project::new();
    let original = format!(
        "{HANDWRITTEN}\n{}\n\n{}\n\n{}\n\n{TRAILING}",
        marker::START_MARKER,
        "- **[`src/**/*.ts`]** An out-of-date rule from a previous sync",
        marker::END_MARKER,
    );
    project.write("CLAUDE.md", &original);

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );

    // Sync repeatedly, adding a rule in between, exactly as a day of work would.
    project.store.sync().unwrap();
    project.confirm(
        RuleCategory::CodeStyle,
        "**/*",
        "Prefer early returns (guard clauses) over nested if-else blocks",
    );
    project.store.sync().unwrap();
    project.store.sync().unwrap();

    let updated = project.read("CLAUDE.md").unwrap();
    assert!(updated.starts_with(HANDWRITTEN), "{updated}");
    assert!(updated.ends_with(TRAILING), "{updated}");
    assert!(
        !updated.contains("out-of-date rule"),
        "the previous block should have been replaced:\n{updated}"
    );
    assert!(updated.contains("@AutoEntity()"), "{updated}");
    assert!(updated.contains("early returns"), "{updated}");
    assert_eq!(
        updated.matches(marker::END_MARKER).count(),
        1,
        "exactly one block should exist:\n{updated}"
    );
}

#[test]
fn syncing_twice_with_no_rule_changes_leaves_every_file_byte_identical() {
    let project = Project::new();
    project.write("CLAUDE.md", HANDWRITTEN);

    project.confirm_with_patterns(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
        "@Entity('users')\nexport class User {}",
        "@AutoEntity()\nexport class User {}",
    );

    let first = project.store.sync().unwrap();
    let before: Vec<(std::path::PathBuf, Vec<u8>)> = first
        .written
        .iter()
        .map(|p| (p.clone(), std::fs::read(p).unwrap()))
        .collect();
    assert!(!before.is_empty());

    let second = project.store.sync().unwrap();
    assert!(
        second.written.is_empty(),
        "a no-op sync must not rewrite anything: {second:#?}"
    );

    for (path, bytes) in before {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "{} changed on a no-op sync",
            path.display()
        );
    }
}

#[test]
fn a_file_with_unbalanced_markers_is_refused_and_left_untouched() {
    let project = Project::new();
    let broken = format!(
        "{HANDWRITTEN}\n{}\n\n- A block someone truncated by hand\n",
        marker::START_MARKER
    );
    project.write("CLAUDE.md", &broken);

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );

    let err = project.store.sync().unwrap_err();
    assert!(
        matches!(err, Error::MalformedMarkers { .. }),
        "expected a refusal, got {err:?}"
    );
    assert!(err.to_string().contains("CLAUDE.md"), "{err}");
    assert_eq!(
        project.read("CLAUDE.md").unwrap(),
        broken,
        "a file we refuse to understand must not be modified"
    );
}

#[test]
fn removing_the_last_rule_removes_the_block_but_not_the_document() {
    let project = Project::new();
    project.write("CLAUDE.md", HANDWRITTEN);

    let rule = project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.store.sync().unwrap();
    assert!(marker::has_block(&project.read("CLAUDE.md").unwrap()));

    project.store.delete_rule(&rule.id).unwrap();
    project.store.sync().unwrap();

    assert_eq!(
        project.read("CLAUDE.md").unwrap(),
        HANDWRITTEN,
        "the document should be exactly as the developer left it"
    );
}

#[test]
fn disabling_every_rule_has_the_same_effect_as_having_none() {
    let project = Project::new();
    project.write("CLAUDE.md", HANDWRITTEN);

    let rule = project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.store.sync().unwrap();

    project.store.set_enabled(&rule.id, false).unwrap();
    project.store.sync().unwrap();

    assert_eq!(project.read("CLAUDE.md").unwrap(), HANDWRITTEN);
    assert!(
        !project.exists(".cursor/rules/opentaste-architecture.mdc"),
        "a disabled rule should leave no Cursor file behind"
    );
}

#[test]
fn a_file_using_windows_line_endings_keeps_them() {
    let project = Project::new();
    project.write("CLAUDE.md", &HANDWRITTEN.replace('\n', "\r\n"));

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.store.sync().unwrap();

    let updated = project.read("CLAUDE.md").unwrap();
    assert!(
        !updated.contains("\n\n"),
        "no bare LF should have been introduced"
    );
    assert!(
        updated.starts_with("# Project Conventions\r\n"),
        "{updated:?}"
    );

    // And a second sync still recognises the block rather than appending another.
    project.store.sync().unwrap();
    let again = project.read("CLAUDE.md").unwrap();
    assert_eq!(again, updated);
    assert_eq!(again.matches(marker::END_MARKER).count(), 1);
}

#[test]
fn clear_all_leaves_the_repository_as_if_opentaste_had_never_run() {
    let project = Project::new();
    for target in marker_targets(&project) {
        project.write(target, HANDWRITTEN);
    }

    project.confirm(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        "Use custom @AutoEntity() decorators instead of raw TypeORM @Entity()",
    );
    project.store.sync().unwrap();

    transpiler::clear_all(&project.paths).unwrap();

    for target in marker_targets(&project) {
        assert_eq!(
            project.read(target).as_deref(),
            Some(HANDWRITTEN),
            "{target} was not restored"
        );
    }
    assert!(!project.exists(".cursor/rules/opentaste-architecture.mdc"));
}
