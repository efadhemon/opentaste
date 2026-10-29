//! The Universal Transpiler: confirmed rules in, editor instruction files out.
//!
//! Two kinds of target exist, and the difference matters:
//!
//! - **Owned files.** `.cursor/rules/opentaste-*.mdc` are generated in full,
//!   so they can be rewritten and deleted freely.
//! - **Shared files.** `CLAUDE.md`, `.windsurfrules`, `AGENTS.md` and
//!   `.github/copilot-instructions.md` belong to the developer. OpenTaste only
//!   ever touches the region between its markers; see [`marker`].
//!
//! Every write goes through [`crate::storage::atomic`], and a write whose
//! content is unchanged is skipped entirely, so a no-op sync leaves no trace.

pub mod agents_md;
pub mod claude_md;
pub mod cursor_mdc;
pub mod marker;
pub mod render;
pub mod windsurf;

use std::path::PathBuf;

use crate::config::Paths;
use crate::error::Result;
use crate::models::TasteRule;

/// What one sync run did to the filesystem.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// Files whose bytes changed.
    pub written: Vec<PathBuf>,
    /// Files that were already correct.
    pub unchanged: Vec<PathBuf>,
    /// Files, or blocks, that were removed because no rule justifies them.
    pub removed: Vec<PathBuf>,
}

impl SyncReport {
    /// Note the result of one write.
    pub(crate) fn record(&mut self, path: PathBuf, changed: bool) {
        if changed {
            self.written.push(path);
        } else {
            self.unchanged.push(path);
        }
    }

    /// Whether this run altered anything at all.
    pub fn is_empty(&self) -> bool {
        self.written.is_empty() && self.removed.is_empty()
    }

    /// Every path this run considered.
    pub fn touched(&self) -> impl Iterator<Item = &PathBuf> {
        self.written
            .iter()
            .chain(self.unchanged.iter())
            .chain(self.removed.iter())
    }
}

/// Compile `rules` into every supported editor's instruction file.
///
/// Only active rules are emitted: enabled, and at or above the promotion
/// confidence. Passing the full rule set is fine, and is what callers normally
/// do; the filtering happens here so every target agrees on what counts.
pub fn sync_all(paths: &Paths, rules: &[TasteRule]) -> Result<SyncReport> {
    let active: Vec<TasteRule> = rules.iter().filter(|r| r.is_active()).cloned().collect();

    let mut report = SyncReport::default();
    cursor_mdc::sync(paths, &active, &mut report)?;
    claude_md::sync(paths, &active, &mut report)?;
    windsurf::sync(paths, &active, &mut report)?;
    agents_md::sync(paths, &active, &mut report)?;
    Ok(report)
}

/// What [`sync_all`] would do with `rules`, without writing anything.
///
/// `written` lists the targets that are stale (a sync would rewrite them),
/// `unchanged` the ones already up to date, and `removed` the ones a sync
/// would delete or strip. `taste status` reports from this.
pub fn check_all(paths: &Paths, rules: &[TasteRule]) -> Result<SyncReport> {
    let active: Vec<TasteRule> = rules.iter().filter(|r| r.is_active()).cloned().collect();

    let mut report = SyncReport::default();
    cursor_mdc::check(paths, &active, &mut report)?;
    let body = render::sections(&active);
    for path in paths.marker_targets() {
        let existing = crate::storage::atomic::read_to_string_opt(&path)?;
        if active.is_empty() {
            let has_block = match &existing {
                Some(existing) => marker::strip(existing, &path)?.is_some(),
                None => false,
            };
            if has_block {
                report.removed.push(path);
            }
            continue;
        }
        let existing = existing.unwrap_or_default();
        let updated = marker::inject(&existing, &body, &path)?;
        report.record(path, updated != existing);
    }
    Ok(report)
}

/// Remove every OpenTaste block and generated file, as `taste uninstall` would.
///
/// Developer-written content in the shared files is preserved.
pub fn clear_all(paths: &Paths) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    cursor_mdc::sync(paths, &[], &mut report)?;
    for path in paths.marker_targets() {
        if marker::clear_block(&path)? {
            report.removed.push(path);
        }
    }
    Ok(report)
}

/// Inject, or withdraw, the OpenTaste block in one developer-owned file.
///
/// With no active rules the block is removed rather than left behind empty, and
/// a file that contained nothing else is deleted. A file that never had a block
/// is not created just to say there is nothing to say.
fn sync_marker_target(path: PathBuf, active: &[TasteRule], report: &mut SyncReport) -> Result<()> {
    if active.is_empty() {
        if marker::clear_block(&path)? {
            report.removed.push(path);
        }
        return Ok(());
    }

    let changed = marker::write_block(&path, &render::sections(active))?;
    report.record(path, changed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RuleCategory, RuleSource};
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

    fn snapshot(root: &std::path::Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push((path.clone(), std::fs::read(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn checking_reports_what_a_sync_would_do_and_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        std::fs::write(paths.claude_md(), "# Mine\n").unwrap();
        let rules = [rule(
            "imports-alias",
            RuleCategory::Imports,
            "**/*.ts",
            "Prefer path aliases",
        )];

        let before = snapshot(tmp.path());
        let pending = check_all(&paths, &rules).unwrap();
        assert_eq!(snapshot(tmp.path()), before, "a check must not write");
        assert!(pending.unchanged.is_empty(), "{pending:?}");
        assert!(pending.written.contains(&paths.claude_md()), "{pending:?}");

        let synced = sync_all(&paths, &rules).unwrap();
        assert_eq!(
            pending.written, synced.written,
            "the check predicts the sync"
        );

        let after = check_all(&paths, &rules).unwrap();
        assert!(
            after.written.is_empty() && after.removed.is_empty(),
            "{after:?}"
        );
        assert_eq!(after.unchanged.len(), synced.written.len());
    }

    #[test]
    fn checking_with_no_rules_reports_the_blocks_a_sync_would_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        let rules = [rule(
            "imports-alias",
            RuleCategory::Imports,
            "**/*.ts",
            "Prefer path aliases",
        )];
        sync_all(&paths, &rules).unwrap();
        let stray = paths.cursor_rules_dir().join("opentaste-retired.mdc");
        std::fs::write(&stray, "old").unwrap();

        let pending = check_all(&paths, &[]).unwrap();
        assert!(pending.removed.contains(&paths.claude_md()), "{pending:?}");
        assert!(pending.removed.contains(&stray), "{pending:?}");
        assert!(
            pending
                .removed
                .contains(&cursor_mdc::file_for(&paths, RuleCategory::Imports)),
            "{pending:?}"
        );
        assert!(
            stray.exists() && paths.claude_md().exists(),
            "a check must not delete"
        );
    }

    #[test]
    fn only_active_rules_reach_the_editor() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());

        let mut disabled = rule(
            "arch-disabled",
            RuleCategory::Architecture,
            "src/**/*.ts",
            "A rule the developer switched off",
        );
        disabled.enabled = false;

        let mut unsure = rule(
            "types-unsure",
            RuleCategory::Types,
            "src/**/*.ts",
            "A rule that has not earned its place",
        );
        unsure.confidence = 0.79;

        let active = rule(
            "style-active",
            RuleCategory::CodeStyle,
            "**/*",
            "Prefer early returns over nested if-else blocks",
        );

        sync_all(&paths, &[disabled, unsure, active]).unwrap();

        let claude = std::fs::read_to_string(paths.claude_md()).unwrap();
        assert!(claude.contains("Prefer early returns"), "{claude}");
        assert!(!claude.contains("switched off"), "{claude}");
        assert!(!claude.contains("not earned"), "{claude}");

        assert!(cursor_mdc::file_for(&paths, RuleCategory::CodeStyle).exists());
        assert!(!cursor_mdc::file_for(&paths, RuleCategory::Architecture).exists());
        assert!(!cursor_mdc::file_for(&paths, RuleCategory::Types).exists());
    }

    #[test]
    fn every_documented_target_is_written() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());

        let report = sync_all(
            &paths,
            &[rule(
                "style-early-return",
                RuleCategory::CodeStyle,
                "**/*",
                "Prefer early returns over nested if-else blocks",
            )],
        )
        .unwrap();

        for expected in [
            paths.claude_md(),
            paths.windsurf_rules(),
            paths.agents_md(),
            paths.copilot_instructions(),
            cursor_mdc::file_for(&paths, RuleCategory::CodeStyle),
        ] {
            assert!(expected.exists(), "{} was not written", expected.display());
            assert!(report.written.contains(&expected), "{report:#?}");
        }
    }

    #[test]
    fn an_empty_rule_set_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());

        let report = sync_all(&paths, &[]).unwrap();

        assert!(report.is_empty(), "{report:#?}");
        assert!(!paths.claude_md().exists());
        assert!(!paths.windsurf_rules().exists());
        assert!(!paths.agents_md().exists());
        assert!(!paths.copilot_instructions().exists());
    }

    #[test]
    fn a_second_sync_changes_nothing_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        let rules = vec![rule(
            "style-early-return",
            RuleCategory::CodeStyle,
            "**/*",
            "Prefer early returns over nested if-else blocks",
        )];

        let first = sync_all(&paths, &rules).unwrap();
        let before: Vec<Vec<u8>> = first
            .written
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();

        let second = sync_all(&paths, &rules).unwrap();
        assert!(second.written.is_empty(), "{second:#?}");
        assert!(second.is_empty(), "{second:#?}");
        assert_eq!(second.unchanged.len(), first.written.len());

        let after: Vec<Vec<u8>> = first
            .written
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn withdrawing_the_last_rule_withdraws_the_block_too() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        std::fs::write(paths.claude_md(), "# My notes\n\nKeep me.\n").unwrap();

        sync_all(
            &paths,
            &[rule(
                "style-early-return",
                RuleCategory::CodeStyle,
                "**/*",
                "Prefer early returns over nested if-else blocks",
            )],
        )
        .unwrap();
        assert!(claude_md::has_block(&paths).unwrap());

        let report = sync_all(&paths, &[]).unwrap();

        assert!(!claude_md::has_block(&paths).unwrap());
        assert!(report.removed.contains(&paths.claude_md()), "{report:#?}");
        assert_eq!(
            std::fs::read_to_string(paths.claude_md()).unwrap(),
            "# My notes\n\nKeep me.\n",
            "the developer's own notes must survive"
        );
        assert!(
            !paths.agents_md().exists(),
            "a file OpenTaste created alone should be removed, not left empty"
        );
    }

    #[test]
    fn clear_all_undoes_everything_opentaste_added() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::for_root(tmp.path());
        std::fs::write(paths.windsurf_rules(), "# Handwritten windsurf rules\n").unwrap();

        sync_all(
            &paths,
            &[rule(
                "arch-entity",
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                "Use custom @AutoEntity() decorators",
            )],
        )
        .unwrap();

        clear_all(&paths).unwrap();

        assert!(!cursor_mdc::file_for(&paths, RuleCategory::Architecture).exists());
        assert!(!paths.claude_md().exists());
        assert!(!paths.agents_md().exists());
        assert_eq!(
            std::fs::read_to_string(paths.windsurf_rules()).unwrap(),
            "# Handwritten windsurf rules\n"
        );
    }
}
