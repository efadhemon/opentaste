//! Shared fixtures for the integration tests.

#![allow(dead_code)]

use chrono::{DateTime, Utc};
use taste_core::config::Paths;
use taste_core::models::{RuleCategory, RuleSource, TasteRule};
use taste_core::storage::{Observation, RuleStore};
use tempfile::TempDir;

/// A throwaway project directory with an open store.
///
/// The `TempDir` is held alongside the store so the directory outlives it.
pub struct Project {
    pub dir: TempDir,
    pub paths: Paths,
    pub store: RuleStore,
}

impl Project {
    /// Create an empty project whose candidate cache lives in memory.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let paths = Paths::for_root(dir.path());
        let store = RuleStore::open_ephemeral(paths.clone()).expect("open store");
        Project { dir, paths, store }
    }

    /// Create a project whose candidate cache is a real `.taste/cache.db`.
    pub fn with_cache_db() -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let paths = Paths::for_root(dir.path());
        let store = RuleStore::open(paths.clone()).expect("open store");
        Project { dir, paths, store }
    }

    /// Read a project-relative file, if it exists.
    pub fn read(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join(relative)).ok()
    }

    /// Write a project-relative file, creating parent directories.
    pub fn write(&self, relative: &str, contents: &str) {
        let path = self.dir.path().join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent");
        }
        std::fs::write(path, contents).expect("write file");
    }

    /// Whether a project-relative path exists.
    pub fn exists(&self, relative: &str) -> bool {
        self.dir.path().join(relative).exists()
    }

    /// Confirm a rule outright, the way explicit chat feedback would.
    pub fn confirm(&self, category: RuleCategory, scope: &str, instruction: &str) -> TasteRule {
        self.store
            .record_observation(Observation::new(
                category,
                scope,
                RuleSource::McpFeedback,
                instruction,
            ))
            .expect("record observation")
            .promoted()
            .expect("explicit feedback promotes immediately")
            .clone()
    }

    /// Confirm a rule and attach its illustrating snippets.
    pub fn confirm_with_patterns(
        &self,
        category: RuleCategory,
        scope: &str,
        instruction: &str,
        anti_pattern: &str,
        preferred_pattern: &str,
    ) -> TasteRule {
        self.confirm_observation(
            Observation::new(category, scope, RuleSource::McpFeedback, instruction).with_patterns(
                Some(anti_pattern.to_string()),
                Some(preferred_pattern.to_string()),
            ),
        )
    }

    /// Confirm a rule described by an existing one, snippets and all.
    pub fn confirm_like(&self, template: &TasteRule) -> TasteRule {
        self.confirm_observation(
            Observation::new(
                template.category,
                &template.scope,
                RuleSource::McpFeedback,
                &template.instruction,
            )
            .with_patterns(
                template.anti_pattern.clone(),
                template.preferred_pattern.clone(),
            ),
        )
    }

    fn confirm_observation(&self, observation: Observation) -> TasteRule {
        self.store
            .record_observation(observation)
            .expect("record observation")
            .promoted()
            .expect("explicit feedback promotes immediately")
            .clone()
    }
}

impl Default for Project {
    fn default() -> Self {
        Project::new()
    }
}

/// A fixed timestamp, so generated output is comparable across runs.
pub fn fixed_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A fully populated rule, for tests that need one without a store.
pub fn rule(id: &str, category: RuleCategory, scope: &str, instruction: &str) -> TasteRule {
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
        updated_at: fixed_time(),
    }
}
