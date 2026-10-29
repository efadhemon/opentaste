//! The git-tracked rule file, `.taste/rules.json`.
//!
//! This file is reviewed in pull requests and merged across branches, so its
//! serialisation is deliberately canonical: rules sorted by identifier,
//! confidence pinned to two decimals, two-space indentation and exactly one
//! trailing newline. Saving an unchanged rule set reproduces the file byte for
//! byte, which is what keeps it out of diffs it has no business appearing in.

use std::path::{Path, PathBuf};

use crate::config::SCHEMA_URL;
use crate::error::{Error, Result};
use crate::models::RulesFile;
use crate::storage::atomic::{read_to_string_opt, write_if_changed};

/// Reader and writer for one project's `.taste/rules.json`.
#[derive(Debug, Clone)]
pub struct JsonStore {
    path: PathBuf,
}

impl JsonStore {
    /// Point the store at a rule file, which need not exist yet.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        JsonStore { path: path.into() }
    }

    /// Path of the backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load the rule set, returning an empty one when the file is absent.
    pub fn load(&self) -> Result<RulesFile> {
        let Some(raw) = read_to_string_opt(&self.path)? else {
            return Ok(RulesFile::default());
        };
        if raw.trim().is_empty() {
            return Ok(RulesFile::default());
        }
        let mut file: RulesFile =
            serde_json::from_str(&raw).map_err(|e| Error::json(&self.path, e))?;
        file.sort();
        Ok(file)
    }

    /// Write the rule set in canonical form.
    ///
    /// Returns `true` when the bytes on disk actually changed.
    pub fn save(&self, rules: &RulesFile) -> Result<bool> {
        write_if_changed(&self.path, self.render(rules)?.as_bytes())
    }

    /// Serialise the rule set exactly as [`JsonStore::save`] would write it.
    ///
    /// The `$schema` URL is always the current one, so a file written when it
    /// pointed somewhere else is corrected the next time it is saved.
    pub fn render(&self, rules: &RulesFile) -> Result<String> {
        let mut canonical = rules.clone();
        canonical.schema = SCHEMA_URL.to_string();
        canonical.sort();
        let mut json =
            serde_json::to_string_pretty(&canonical).map_err(|e| Error::json(&self.path, e))?;
        json.push('\n');
        Ok(json)
    }

    /// Whether the rule file exists on disk.
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RuleCategory, RuleSource, TasteRule};
    use chrono::{DateTime, Utc};
    use pretty_assertions::assert_eq;

    fn rule(id: &str) -> TasteRule {
        TasteRule {
            id: id.to_string(),
            category: RuleCategory::Architecture,
            scope: "src/**/*.ts".to_string(),
            confidence: 0.95,
            occurrences: 4,
            enabled: true,
            source: RuleSource::AstDiff,
            instruction: format!("Instruction for {id}"),
            anti_pattern: None,
            preferred_pattern: None,
            updated_at: DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn loading_an_absent_file_yields_an_empty_rule_set() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join(".taste").join("rules.json"));

        assert!(!store.exists());
        assert_eq!(store.load().unwrap(), RulesFile::default());
    }

    #[test]
    fn rules_are_written_sorted_by_id_whatever_order_they_arrive_in() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join("rules.json"));

        let file = RulesFile::with_rules([rule("z-last"), rule("a-first"), rule("m-middle")]);
        store.save(&file).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(
            loaded
                .rules
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-first", "m-middle", "z-last"]
        );
    }

    #[test]
    fn saving_an_unchanged_rule_set_does_not_touch_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join("rules.json"));

        let file = RulesFile::with_rules([rule("a"), rule("b")]);

        assert!(store.save(&file).unwrap(), "first save should write");
        let first = std::fs::read(store.path()).unwrap();

        assert!(!store.save(&file).unwrap(), "second save should be a no-op");
        let second = std::fs::read(store.path()).unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn the_rendered_file_ends_with_exactly_one_newline() {
        let store = JsonStore::new("/tmp/unused.json");
        let rendered = store.render(&RulesFile::default()).unwrap();
        assert!(rendered.ends_with("}\n"), "{rendered}");
        assert!(!rendered.ends_with("\n\n"), "{rendered}");
    }

    #[test]
    fn the_spec_field_names_survive_a_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join("rules.json"));

        let file = RulesFile::with_rules([rule("arch-thing")]);
        store.save(&file).unwrap();

        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(raw.contains(r#""$schema""#), "{raw}");
        assert!(raw.contains(r#""project_context""#), "{raw}");
        assert!(raw.contains(r#""confidence": 0.95"#), "{raw}");

        assert_eq!(store.load().unwrap(), file);
    }

    #[test]
    fn saving_replaces_an_outdated_schema_url_with_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let store = JsonStore::new(tmp.path().join("rules.json"));
        let mut file = RulesFile::with_rules([rule("arch-thing")]);
        file.schema = "https://opentaste.dev/schema/v1.json".to_string();
        store.save(&file).unwrap();

        let raw = std::fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains("opentaste.dev"), "{raw}");
        assert_eq!(store.load().unwrap().schema, SCHEMA_URL);
    }

    #[test]
    fn an_empty_file_is_treated_as_an_empty_rule_set() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rules.json");
        std::fs::write(&path, "   \n").unwrap();

        assert_eq!(JsonStore::new(&path).load().unwrap(), RulesFile::default());
    }

    #[test]
    fn malformed_json_names_the_offending_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("rules.json");
        std::fs::write(&path, "{ not json").unwrap();

        let err = JsonStore::new(&path).load().unwrap_err();
        assert!(matches!(err, Error::Json { .. }));
        assert!(err.to_string().contains("rules.json"), "{err}");
    }
}
