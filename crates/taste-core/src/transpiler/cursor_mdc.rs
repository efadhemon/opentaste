//! Scoped Cursor rule files, `.cursor/rules/opentaste-<category>.mdc`.
//!
//! Unlike the marker targets, these files are generated in full and owned
//! entirely by OpenTaste, which is why the frontmatter says so. One file per
//! category keeps each set of rules attachable to just the globs it concerns.

use std::path::{Path, PathBuf};

use crate::config::Paths;
use crate::error::Result;
use crate::models::{RuleCategory, TasteRule};
use crate::storage::atomic::{remove_if_present, write_if_changed};
use crate::transpiler::render::{self, MATCH_ALL_SCOPE};
use crate::transpiler::SyncReport;

/// Filename prefix identifying a generated Cursor rule file.
pub const FILE_PREFIX: &str = "opentaste-";
/// Extension Cursor expects for rule files.
pub const FILE_EXTENSION: &str = "mdc";

/// Path of the generated file for one category.
pub fn file_for(paths: &Paths, category: RuleCategory) -> PathBuf {
    paths
        .cursor_rules_dir()
        .join(format!("{FILE_PREFIX}{}.{FILE_EXTENSION}", category.slug()))
}

/// Write one `.mdc` file per category and delete the ones no longer earned.
pub fn sync(paths: &Paths, active: &[TasteRule], report: &mut SyncReport) -> Result<()> {
    let grouped = render::group_by_category(active);

    for (category, rules) in &grouped {
        let path = file_for(paths, *category);
        let changed = write_if_changed(&path, render_file(*category, rules).as_bytes())?;
        report.record(path, changed);
    }

    for category in RuleCategory::ALL {
        if grouped.contains_key(&category) {
            continue;
        }
        let path = file_for(paths, category);
        if remove_if_present(&path)? {
            report.removed.push(path);
        }
    }

    prune_unknown(&paths.cursor_rules_dir(), report)?;
    Ok(())
}

/// Remove generated files whose category this build no longer recognises.
///
/// Without this, renaming or retiring a category would strand a file that the
/// editor keeps reading forever, with nothing left to regenerate or delete it.
fn prune_unknown(dir: &Path, report: &mut SyncReport) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };

    let known: Vec<String> = RuleCategory::ALL
        .iter()
        .map(|c| format!("{FILE_PREFIX}{}.{FILE_EXTENSION}", c.slug()))
        .collect();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_ours = name.starts_with(FILE_PREFIX) && name.ends_with(FILE_EXTENSION);
        if is_ours && !known.contains(&name) {
            let path = entry.path();
            if remove_if_present(&path)? {
                report.removed.push(path);
            }
        }
    }
    Ok(())
}

/// Render the complete contents of one category's `.mdc` file.
pub fn render_file(category: RuleCategory, rules: &[&TasteRule]) -> String {
    let scopes = render::scopes_of(rules);
    let always_apply = scopes.iter().any(|s| s == MATCH_ALL_SCOPE);

    let mut out = String::from("---\n");
    out.push_str(&format!(
        "description: OpenTaste auto-learned rules for {} (Do not edit manually)\n",
        category.title()
    ));
    // When a rule covers the whole project there is nothing to scope against,
    // so the file is always in context and listing globs would only mislead.
    if always_apply {
        out.push_str("alwaysApply: true\n");
    } else {
        out.push_str(&format!("globs: {}\n", render_globs(&scopes)));
        out.push_str("alwaysApply: false\n");
    }
    out.push_str("---\n\n");
    out.push_str(&format!(
        "# OpenTaste: {} Preferences\n\n",
        category.title()
    ));
    out.push_str(&render::category_body(rules));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Render a glob list as a YAML flow sequence.
///
/// Serialising through JSON gets the quoting right for globs containing commas,
/// brackets or quotes, and JSON flow sequences are valid YAML.
fn render_globs(scopes: &[String]) -> String {
    serde_json::to_string(scopes).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleSource;
    use chrono::{DateTime, Utc};

    fn rule(id: &str, category: RuleCategory, scope: &str, instruction: &str) -> TasteRule {
        TasteRule {
            id: id.to_string(),
            category,
            scope: scope.to_string(),
            confidence: 0.95,
            occurrences: 4,
            enabled: true,
            source: RuleSource::AstDiff,
            instruction: instruction.to_string(),
            anti_pattern: None,
            preferred_pattern: None,
            updated_at: DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn the_filename_follows_the_category_slug() {
        let paths = Paths::for_root("/proj");
        assert_eq!(
            file_for(&paths, RuleCategory::ErrorHandling),
            Path::new("/proj/.cursor/rules/opentaste-error-handling.mdc")
        );
    }

    #[test]
    fn frontmatter_lists_the_sorted_union_of_the_categorys_scopes() {
        let rules = [
            rule(
                "b",
                RuleCategory::Architecture,
                "src/**/*.service.ts",
                "Service rule",
            ),
            rule(
                "a",
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                "Entity rule",
            ),
        ];
        let refs: Vec<&TasteRule> = rules.iter().collect();

        let rendered = render_file(RuleCategory::Architecture, &refs);
        assert!(
            rendered.contains(r#"globs: ["src/**/*.entity.ts","src/**/*.service.ts"]"#),
            "{rendered}"
        );
        assert!(rendered.contains("alwaysApply: false"), "{rendered}");
        assert!(
            rendered.starts_with("---\ndescription: OpenTaste auto-learned rules for Architecture"),
            "{rendered}"
        );
    }

    #[test]
    fn a_project_wide_rule_switches_the_file_to_always_apply() {
        let rules = [rule(
            "style-early-return",
            RuleCategory::CodeStyle,
            "**/*",
            "Prefer early returns",
        )];
        let refs: Vec<&TasteRule> = rules.iter().collect();

        let rendered = render_file(RuleCategory::CodeStyle, &refs);
        assert!(rendered.contains("alwaysApply: true"), "{rendered}");
        assert!(
            !rendered.contains("globs:"),
            "globs alongside alwaysApply would only mislead: {rendered}"
        );
    }

    #[test]
    fn a_category_with_no_rules_has_its_file_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        let mut report = SyncReport::default();

        let rules = vec![rule(
            "arch-entity",
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            "Entity rule",
        )];
        sync(&paths, &rules, &mut report).unwrap();
        assert!(file_for(&paths, RuleCategory::Architecture).exists());

        let mut second = SyncReport::default();
        sync(&paths, &[], &mut second).unwrap();
        assert!(!file_for(&paths, RuleCategory::Architecture).exists());
        assert_eq!(
            second.removed,
            vec![file_for(&paths, RuleCategory::Architecture)]
        );
    }

    #[test]
    fn a_generated_file_for_an_unrecognised_category_is_cleaned_up() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        std::fs::create_dir_all(paths.cursor_rules_dir()).unwrap();

        let stale = paths.cursor_rules_dir().join("opentaste-retired.mdc");
        let handwritten = paths.cursor_rules_dir().join("my-own-rules.mdc");
        std::fs::write(&stale, "stale").unwrap();
        std::fs::write(&handwritten, "mine").unwrap();

        let mut report = SyncReport::default();
        sync(&paths, &[], &mut report).unwrap();

        assert!(!stale.exists(), "our own stale file should go");
        assert!(
            handwritten.exists(),
            "a developer's own rule file must never be touched"
        );
    }

    #[test]
    fn re_running_sync_reports_the_files_as_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        let rules = vec![rule(
            "arch-entity",
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            "Entity rule",
        )];

        let mut first = SyncReport::default();
        sync(&paths, &rules, &mut first).unwrap();
        assert_eq!(first.written.len(), 1);
        assert!(first.unchanged.is_empty());

        let mut second = SyncReport::default();
        sync(&paths, &rules, &mut second).unwrap();
        assert!(second.written.is_empty());
        assert_eq!(second.unchanged.len(), 1);
    }
}
