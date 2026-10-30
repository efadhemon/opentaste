//! Persistence, and the one place rules are allowed to change.
//!
//! Confirmed rules live in a git-tracked JSON file and candidates live in a
//! local SQLite database. [`RuleStore`] owns both so that promotion and
//! demotion happen as a single supervised step rather than as two independent
//! writes that could disagree with each other.

pub mod atomic;
pub mod json_store;
pub mod lock;
pub mod sqlite_store;

use chrono::Utc;
use globset::{Glob, GlobBuilder, GlobMatcher};

use crate::config::Paths;
use crate::error::{Error, Result};
use crate::models::{
    normalize_instruction, rule_id, CandidateRule, ProjectContext, RuleCategory, RuleSource,
    RulesFile, TasteRule,
};
use crate::scorer::{confidence as scoring, dedup};

pub use json_store::JsonStore;
pub use lock::TasteLock;
pub use sqlite_store::{FileSnapshot, SqliteStore};

/// A single sighting of a coding preference, from any of the learning loops.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub category: RuleCategory,
    /// Glob the preference applies to, e.g. `src/**/*.controller.ts`.
    pub scope: String,
    pub source: RuleSource,
    pub instruction: String,
    pub anti_pattern: Option<String>,
    pub preferred_pattern: Option<String>,
    /// Confidence to start at, overriding the per-source default.
    ///
    /// The cold-start scanner sets this to the ratio it measured across the
    /// codebase; the other loops leave it unset.
    pub confidence_override: Option<f32>,
}

impl Observation {
    /// Build an observation with the default confidence for its source.
    pub fn new(
        category: RuleCategory,
        scope: impl Into<String>,
        source: RuleSource,
        instruction: impl Into<String>,
    ) -> Self {
        Observation {
            category,
            scope: scope.into(),
            source,
            instruction: instruction.into(),
            anti_pattern: None,
            preferred_pattern: None,
            confidence_override: None,
        }
    }

    /// Attach the snippets illustrating what to avoid and what to write.
    pub fn with_patterns(
        mut self,
        anti_pattern: Option<String>,
        preferred_pattern: Option<String>,
    ) -> Self {
        self.anti_pattern = anti_pattern;
        self.preferred_pattern = preferred_pattern;
        self
    }

    /// Start from a measured ratio instead of the per-source default.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence_override = Some(confidence);
        self
    }

    fn into_candidate(self) -> CandidateRule {
        let mut candidate =
            CandidateRule::new(self.category, self.scope, self.source, self.instruction)
                .with_patterns(self.anti_pattern, self.preferred_pattern);
        if let Some(confidence) = self.confidence_override {
            candidate = candidate.with_confidence(confidence);
        }
        candidate
    }
}

/// What recording an observation did to the rule set.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A new candidate was created but has not earned promotion yet.
    Created { id: String, confidence: f32 },
    /// An existing candidate gained an occurrence and some confidence.
    Reinforced {
        id: String,
        confidence: f32,
        occurrences: u32,
    },
    /// A rule crossed both thresholds and now lives in `.taste/rules.json`.
    Promoted(Box<TasteRule>),
    /// The preference was already a confirmed rule, which was reinforced.
    AlreadyConfirmed {
        id: String,
        confidence: f32,
        occurrences: u32,
    },
    /// The observation is the opposite of the confirmed rule `conflicts_with`
    /// ("prefer `type`" against "prefer `interface`"). Nothing was recorded:
    /// installing both would put two contradicting instructions in front of
    /// the editor. `id` is the identifier the observation would have had.
    Conflicts { id: String, conflicts_with: String },
}

impl Outcome {
    /// Identifier of the rule or candidate this outcome concerns.
    pub fn id(&self) -> &str {
        match self {
            Outcome::Created { id, .. }
            | Outcome::Reinforced { id, .. }
            | Outcome::AlreadyConfirmed { id, .. }
            | Outcome::Conflicts { id, .. } => id,
            Outcome::Promoted(rule) => &rule.id,
        }
    }

    /// The rule, if this outcome promoted one. Used to decide whether the
    /// editor should be told something was learned.
    pub fn promoted(&self) -> Option<&TasteRule> {
        match self {
            Outcome::Promoted(rule) => Some(rule),
            _ => None,
        }
    }
}

/// The whole rule set for one project: confirmed rules plus local candidates.
#[derive(Debug)]
pub struct RuleStore {
    paths: Paths,
    json: JsonStore,
    candidates: SqliteStore,
}

impl RuleStore {
    /// Open (creating as needed) the stores for the project at `paths`.
    pub fn open(paths: Paths) -> Result<Self> {
        let json = JsonStore::new(paths.rules_json.clone());
        let candidates = SqliteStore::open(&paths.cache_db)?;
        Ok(RuleStore {
            paths,
            json,
            candidates,
        })
    }

    /// Open a store whose candidate database lives only in memory.
    ///
    /// The JSON file is still real, which is what integration tests want: the
    /// durable, reviewable half on disk and the disposable half in memory.
    pub fn open_ephemeral(paths: Paths) -> Result<Self> {
        let json = JsonStore::new(paths.rules_json.clone());
        Ok(RuleStore {
            paths,
            json,
            candidates: SqliteStore::open_in_memory()?,
        })
    }

    /// Resolved paths for this project.
    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    /// The candidate database, for callers needing snapshot access.
    pub fn candidates(&self) -> &SqliteStore {
        &self.candidates
    }

    /// Hold the project lock for the duration of a caller-managed operation.
    pub fn lock(&self) -> Result<TasteLock> {
        TasteLock::acquire(&self.paths.lock_file)
    }

    /// Load the confirmed rule set.
    pub fn rules(&self) -> Result<RulesFile> {
        self.json.load()
    }

    /// Confirmed rules eligible for compilation into editor instruction files.
    pub fn active_rules(&self) -> Result<Vec<TasteRule>> {
        Ok(self
            .rules()?
            .rules
            .into_iter()
            .filter(TasteRule::is_active)
            .collect())
    }

    /// All unpromoted candidates, ordered by identifier.
    pub fn candidate_rules(&self) -> Result<Vec<CandidateRule>> {
        self.candidates.list_candidates()
    }

    /// Active rules whose scope glob matches `file_path`.
    ///
    /// `file_path` is interpreted relative to the project root; absolute paths
    /// inside the project are relativised first.
    pub fn rules_for_file(&self, file_path: impl AsRef<std::path::Path>) -> Result<Vec<TasteRule>> {
        let path = file_path.as_ref();
        let relative = self.paths.relativize(path);
        let normalized = normalize_path(relative);

        let mut matched = Vec::new();
        for rule in self.active_rules()? {
            if scope_matcher(&rule.scope)?.is_match(&normalized) {
                matched.push(rule);
            }
        }
        Ok(matched)
    }

    /// Enabled confirmed rules whose scope glob matches `file_path`, including
    /// those that have fallen below the promotion confidence.
    ///
    /// Contradiction checks use this rather than [`Self::rules_for_file`]: a
    /// rule that drops below 0.80 leaves the editor files but stays in the
    /// rule file, and only further contradictions can take it on down to the
    /// demotion threshold. A disabled rule is the developer's call and is not
    /// returned.
    pub fn enabled_rules_for_file(
        &self,
        file_path: impl AsRef<std::path::Path>,
    ) -> Result<Vec<TasteRule>> {
        let relative = self.paths.relativize(file_path.as_ref());
        let normalized = normalize_path(relative);

        let mut matched = Vec::new();
        for rule in self.rules()?.rules.into_iter().filter(|rule| rule.enabled) {
            if scope_matcher(&rule.scope)?.is_match(&normalized) {
                matched.push(rule);
            }
        }
        Ok(matched)
    }

    /// Record one sighting of a coding preference.
    ///
    /// This is the single entry point for the AST scanner, the structural diff
    /// learner and the MCP server. It deduplicates against what is already
    /// known, applies the scoring model, and promotes across the storage
    /// boundary when both thresholds are met.
    pub fn record_observation(&self, observation: Observation) -> Result<Outcome> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;

        if let Some(existing_id) = find_matching_rule(&rules, &observation) {
            return self.reinforce_confirmed(&mut rules, &existing_id);
        }
        if let Some(conflicts_with) = find_opposite_rule(&rules, &observation) {
            return Ok(Outcome::Conflicts {
                id: rule_id(
                    observation.category,
                    &observation.scope,
                    &observation.instruction,
                ),
                conflicts_with,
            });
        }

        self.decay_opposite_candidates(&observation)?;
        let candidate = match self.find_matching_candidate(&observation)? {
            Some(existing) => merge_into_candidate(existing, &observation),
            None => observation.into_candidate(),
        };

        if scoring::is_promotable(&candidate) {
            return self.promote(&mut rules, candidate);
        }

        let outcome = if candidate.occurrences > 1 {
            Outcome::Reinforced {
                id: candidate.id.clone(),
                confidence: candidate.confidence,
                occurrences: candidate.occurrences,
            }
        } else {
            Outcome::Created {
                id: candidate.id.clone(),
                confidence: candidate.confidence,
            }
        };
        self.candidates.upsert_candidate(&candidate)?;
        Ok(outcome)
    }

    /// Promote a candidate by identifier, as the terminal dashboard's `[p]` key
    /// does. Returns `None` when no such candidate exists.
    pub fn promote_candidate(&self, id: &str) -> Result<Option<TasteRule>> {
        let _guard = self.lock()?;
        let Some(candidate) = self.candidates.get_candidate(id)? else {
            return Ok(None);
        };
        let mut rules = self.json.load()?;
        match self.promote(&mut rules, candidate)? {
            Outcome::Promoted(rule) => Ok(Some(*rule)),
            _ => Ok(None),
        }
    }

    /// Apply the contradiction penalty to a rule the developer just worked
    /// against, demoting it out of the git-tracked file if it falls far enough.
    ///
    /// Returns `false` when no rule or candidate carries that identifier.
    pub fn apply_contradiction(&self, id: &str) -> Result<bool> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;

        if let Some(rule) = rules.find_mut(id) {
            rule.confidence = scoring::decay(rule.confidence);
            rule.updated_at = Utc::now();

            if scoring::should_demote(rule.confidence) {
                let demoted = rules
                    .remove(id)
                    .expect("rule was just located by the same id");
                let mut candidate = demoted.into_candidate();
                candidate.contradictions += 1;
                self.json.save(&rules)?;
                self.candidates.upsert_candidate(&candidate)?;
            } else {
                self.json.save(&rules)?;
            }
            return Ok(true);
        }

        if let Some(mut candidate) = self.candidates.get_candidate(id)? {
            candidate.confidence = scoring::decay(candidate.confidence);
            candidate.contradictions += 1;
            candidate.updated_at = Utc::now();
            self.candidates.upsert_candidate(&candidate)?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Enable or disable a confirmed rule without deleting it.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;
        let Some(rule) = rules.find_mut(id) else {
            return Ok(false);
        };
        if rule.enabled == enabled {
            return Ok(true);
        }
        rule.enabled = enabled;
        rule.updated_at = Utc::now();
        self.json.save(&rules)?;
        Ok(true)
    }

    /// Remove a rule whose evidence is gone, from both the confirmed file and
    /// the candidate cache, *without* remembering it as rejected: the next
    /// scan that finds the evidence again records it again. This is for rules
    /// read from configuration, such as strict null checks when a
    /// `tsconfig.json` turns them off.
    ///
    /// Returns `false` when no rule or candidate carries that identifier.
    pub fn retract_rule(&self, id: &str) -> Result<bool> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;
        let removed_rule = rules.remove(id).is_some();
        if removed_rule {
            self.json.save(&rules)?;
        }
        let removed_candidate = self.candidates.delete_candidate(id)?;
        Ok(removed_rule || removed_candidate)
    }

    /// Remove a rule entirely, from both the confirmed file and the candidate
    /// cache, so it cannot immediately reappear.
    ///
    /// The identifier is also remembered as rejected, so `taste scan` does not
    /// reinstall it from the same ratio it was inferred from.
    pub fn delete_rule(&self, id: &str) -> Result<bool> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;
        let removed_rule = rules.remove(id).is_some();
        if removed_rule {
            self.json.save(&rules)?;
        }
        let removed_candidate = self.candidates.delete_candidate(id)?;
        let removed = removed_rule || removed_candidate;
        if removed {
            self.candidates.reject(id)?;
        }
        Ok(removed)
    }

    /// Create `.taste/rules.json` holding an empty rule set when it does not
    /// exist yet, as `taste init` does. An existing file is never touched.
    ///
    /// Returns `true` when the file was created.
    pub fn ensure_rules_file(&self) -> Result<bool> {
        let _guard = self.lock()?;
        if self.json.exists() {
            return Ok(false);
        }
        self.json.save(&RulesFile::default())?;
        Ok(true)
    }

    /// Replace the recorded project context, as `taste scan` does.
    ///
    /// When the stack and file count are what the file already says, nothing
    /// is written: `rules.json` is git-tracked, and a fresh `last_updated` on
    /// every scan would make it change on every run with nothing to review.
    pub fn set_project_context(&self, context: ProjectContext) -> Result<()> {
        let _guard = self.lock()?;
        let mut rules = self.json.load()?;
        let current = &rules.project_context;
        let same = current.detected_stack == context.detected_stack
            && current.total_files_scanned == context.total_files_scanned;
        if same && self.json.exists() {
            return Ok(());
        }
        rules.project_context = context;
        self.json.save(&rules)?;
        Ok(())
    }

    /// Compile the active rules into every supported editor's instruction file.
    pub fn sync(&self) -> Result<crate::transpiler::SyncReport> {
        let _guard = self.lock()?;
        let rules = self.json.load()?;
        crate::transpiler::sync_all(&self.paths, &rules.rules)
    }

    /// Reinforce a rule that is already confirmed.
    fn reinforce_confirmed(&self, rules: &mut RulesFile, id: &str) -> Result<Outcome> {
        let rule = rules.find_mut(id).ok_or_else(|| Error::InvalidEnumValue {
            kind: "confirmed rule id",
            value: id.to_string(),
        })?;
        rule.occurrences = rule.occurrences.saturating_add(1);
        rule.confidence = scoring::reinforce(rule.confidence);
        rule.updated_at = Utc::now();

        let outcome = Outcome::AlreadyConfirmed {
            id: rule.id.clone(),
            confidence: rule.confidence,
            occurrences: rule.occurrences,
        };
        self.json.save(rules)?;
        Ok(outcome)
    }

    /// Move a candidate into the git-tracked rule file.
    ///
    /// The JSON file is written *before* the candidate row is dropped. If the
    /// process dies in between, the worst outcome is a stale candidate that the
    /// next observation reconciles, rather than a rule that exists nowhere.
    fn promote(&self, rules: &mut RulesFile, candidate: CandidateRule) -> Result<Outcome> {
        let candidate_id = candidate.id.clone();
        let rule = candidate.into_rule();
        rules.upsert(rule.clone());
        self.json.save(rules)?;
        self.candidates.delete_candidate(&candidate_id)?;
        Ok(Outcome::Promoted(Box::new(rule)))
    }

    /// A correction towards one side of a choice is evidence against a
    /// candidate for the other side in the same scope, so that candidate
    /// decays as a contradicted rule would. Without this, a team that moved
    /// from one style to the other keeps both candidates, and whichever
    /// collected six sightings first, old or new, would win; reading history
    /// oldest first makes that common.
    fn decay_opposite_candidates(&self, observation: &Observation) -> Result<()> {
        let opposites: Vec<String> = crate::ast::conflicting_instructions(&observation.instruction)
            .into_iter()
            .map(normalize_instruction)
            .collect();
        if opposites.is_empty() {
            return Ok(());
        }
        for mut candidate in self.candidates.list_candidates_in(observation.category)? {
            if candidate.scope.trim() != observation.scope.trim()
                || !opposites.contains(&normalize_instruction(&candidate.instruction))
            {
                continue;
            }
            candidate.confidence = scoring::decay(candidate.confidence);
            candidate.contradictions += 1;
            candidate.updated_at = Utc::now();
            self.candidates.upsert_candidate(&candidate)?;
        }
        Ok(())
    }

    /// Find a candidate describing the same preference as `observation`.
    fn find_matching_candidate(&self, observation: &Observation) -> Result<Option<CandidateRule>> {
        let expected_id = rule_id(
            observation.category,
            &observation.scope,
            &observation.instruction,
        );
        if let Some(exact) = self.candidates.get_candidate(&expected_id)? {
            return Ok(Some(exact));
        }

        Ok(self
            .candidates
            .list_candidates_in(observation.category)?
            .into_iter()
            .find(|candidate| {
                dedup::is_same_pattern(
                    dedup::Pattern {
                        category: candidate.category,
                        scope: &candidate.scope,
                        instruction: &candidate.instruction,
                        anti_pattern: candidate.anti_pattern.as_deref(),
                        preferred_pattern: candidate.preferred_pattern.as_deref(),
                    },
                    observed_pattern(observation),
                )
            }))
    }
}

/// A confirmed rule that says the opposite of `observation`, for the
/// symmetric choices the analyzers know both sides of.
///
/// Matched on instruction, whatever the scope: a project-wide scanned rule and
/// a package-scoped correction saying the opposite overlap, and refusing an
/// opposite that might not have collided costs less than installing one that
/// does.
fn find_opposite_rule(rules: &RulesFile, observation: &Observation) -> Option<String> {
    let opposites: Vec<String> = crate::ast::conflicting_instructions(&observation.instruction)
        .into_iter()
        .map(normalize_instruction)
        .collect();
    rules
        .rules
        .iter()
        .find(|rule| {
            rule.category == observation.category
                && opposites.contains(&normalize_instruction(&rule.instruction))
        })
        .map(|rule| rule.id.clone())
}

fn observed_pattern(observation: &Observation) -> dedup::Pattern<'_> {
    dedup::Pattern {
        category: observation.category,
        scope: &observation.scope,
        instruction: &observation.instruction,
        anti_pattern: observation.anti_pattern.as_deref(),
        preferred_pattern: observation.preferred_pattern.as_deref(),
    }
}

/// Find a confirmed rule describing the same preference as `observation`.
fn find_matching_rule(rules: &RulesFile, observation: &Observation) -> Option<String> {
    let expected_id = rule_id(
        observation.category,
        &observation.scope,
        &observation.instruction,
    );
    if rules.find(&expected_id).is_some() {
        return Some(expected_id);
    }
    rules
        .rules
        .iter()
        .find(|rule| {
            dedup::is_same_pattern(
                dedup::Pattern {
                    category: rule.category,
                    scope: &rule.scope,
                    instruction: &rule.instruction,
                    anti_pattern: rule.anti_pattern.as_deref(),
                    preferred_pattern: rule.preferred_pattern.as_deref(),
                },
                observed_pattern(observation),
            )
        })
        .map(|rule| rule.id.clone())
}

/// Fold a repeat sighting into an existing candidate.
fn merge_into_candidate(mut candidate: CandidateRule, observation: &Observation) -> CandidateRule {
    candidate.occurrences = candidate.occurrences.saturating_add(1);
    candidate.confidence = scoring::reinforce(candidate.confidence);
    candidate.updated_at = Utc::now();

    // The developer stating a preference outright outranks whatever history the
    // candidate has, including a run of contradictions that previously demoted
    // it. Lifting confidence back to the explicit baseline is what lets someone
    // say "no, really, do it this way" and be believed.
    if observation.source.is_explicit() {
        candidate.source = observation.source;
        candidate.confidence = candidate
            .confidence
            .max(scoring::initial_confidence(observation.source));
    }

    // Later sightings carry fresher examples; only fill gaps, never discard.
    if candidate.anti_pattern.is_none() {
        candidate.anti_pattern = observation.anti_pattern.clone();
    }
    if candidate.preferred_pattern.is_none() {
        candidate.preferred_pattern = observation.preferred_pattern.clone();
    }

    candidate
}

/// Compile a scope glob into a matcher.
///
/// `literal_separator` keeps a single `*` from spilling across directory
/// boundaries, so `src/*.ts` means what a developer reading it expects.
pub(crate) fn scope_matcher(scope: &str) -> Result<GlobMatcher> {
    let glob = GlobBuilder::new(scope)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::glob(scope, e))?;
    Ok(glob.compile_matcher())
}

/// Validate a glob without building a matcher, for input coming from the MCP
/// server or a hand-edited rule file.
pub fn validate_scope(scope: &str) -> Result<()> {
    Glob::new(scope).map_err(|e| Error::glob(scope, e))?;
    Ok(())
}

/// Present a path with forward slashes so globs behave identically on Windows.
fn normalize_path(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleCategory;

    fn store(root: &std::path::Path) -> RuleStore {
        RuleStore::open_ephemeral(Paths::for_root(root)).unwrap()
    }

    fn ast_diff(instruction: &str) -> Observation {
        Observation::new(
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            RuleSource::AstDiff,
            instruction,
        )
    }

    #[test]
    fn scope_globs_match_the_paths_a_developer_would_expect() {
        let deep = scope_matcher("src/**/*.entity.ts").unwrap();
        assert!(deep.is_match("src/users/user.entity.ts"));
        assert!(deep.is_match("src/a/b/c/thing.entity.ts"));
        assert!(!deep.is_match("src/users/user.service.ts"));
        assert!(!deep.is_match("test/user.entity.ts"));

        let shallow = scope_matcher("src/*.ts").unwrap();
        assert!(shallow.is_match("src/main.ts"));
        assert!(
            !shallow.is_match("src/nested/main.ts"),
            "a single star must not cross a directory boundary"
        );

        let everything = scope_matcher("**/*").unwrap();
        assert!(everything.is_match("src/deep/file.ts"));
        assert!(
            everything.is_match("README.md"),
            "the project-wide glob must cover root-level files"
        );
    }

    #[test]
    fn an_invalid_scope_names_the_offending_pattern() {
        let err = validate_scope("src/[unclosed").unwrap_err();
        assert!(err.to_string().contains("src/[unclosed"), "{err}");
    }

    #[test]
    fn a_first_sighting_becomes_a_candidate_not_a_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let outcome = store
            .record_observation(ast_diff("Use @AutoEntity instead of raw @Entity"))
            .unwrap();

        assert!(matches!(outcome, Outcome::Created { .. }));
        assert!(store.rules().unwrap().rules.is_empty());
        assert_eq!(store.candidate_rules().unwrap().len(), 1);
        assert!(!store.json.exists(), "nothing should be committed yet");
    }

    #[test]
    fn repeat_sightings_reinforce_one_candidate_then_promote_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        let instruction = "Use @AutoEntity instead of raw @Entity";

        let mut outcomes = Vec::new();
        for _ in 0..6 {
            outcomes.push(store.record_observation(ast_diff(instruction)).unwrap());
        }

        // Five sightings short of the confidence bar, then promotion on the sixth.
        for outcome in &outcomes[..5] {
            assert!(
                matches!(
                    outcome,
                    Outcome::Created { .. } | Outcome::Reinforced { .. }
                ),
                "{outcome:?}"
            );
        }
        let promoted = outcomes[5].promoted().expect("sixth sighting promotes");
        assert_eq!(promoted.occurrences, 6);
        assert_eq!(promoted.confidence, 0.82);
        assert!(promoted.enabled);

        // The candidate row is gone, so the rule now has exactly one home.
        assert_eq!(store.candidate_rules().unwrap().len(), 0);
        assert_eq!(store.rules().unwrap().rules.len(), 1);
    }

    #[test]
    fn a_rephrased_sighting_reinforces_rather_than_forking_a_new_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        store
            .record_observation(ast_diff("Use the ApiResponse wrapper in controllers"))
            .unwrap();
        store
            .record_observation(ast_diff(
                "Always use the ApiResponse wrapper in controllers",
            ))
            .unwrap();

        let candidates = store.candidate_rules().unwrap();
        assert_eq!(candidates.len(), 1, "{candidates:#?}");
        assert_eq!(candidates[0].occurrences, 2);
    }

    #[test]
    fn explicit_feedback_is_confirmed_on_its_first_sighting() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let outcome = store
            .record_observation(
                Observation::new(
                    RuleCategory::Architecture,
                    "src/**/*.controller.ts",
                    RuleSource::McpFeedback,
                    "Always return a standardized ApiResponse wrapper from controllers",
                )
                .with_patterns(
                    Some("return this.userService.findAll();".to_string()),
                    Some("return ApiResponse.ok(await this.userService.findAll());".to_string()),
                ),
            )
            .unwrap();

        let rule = outcome.promoted().expect("explicit feedback promotes");
        assert_eq!(rule.confidence, 0.95);
        assert_eq!(rule.source, RuleSource::McpFeedback);
        assert_eq!(
            rule.anti_pattern.as_deref(),
            Some("return this.userService.findAll();")
        );
        assert_eq!(store.candidate_rules().unwrap().len(), 0);
    }

    #[test]
    fn a_cold_start_ratio_promotes_straight_away_when_it_is_decisive() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let decisive = store
            .record_observation(
                Observation::new(
                    RuleCategory::Imports,
                    "src/**/*.ts",
                    RuleSource::AstScan,
                    "Import from the @/ alias rather than deep relative paths",
                )
                .with_confidence(0.94),
            )
            .unwrap();
        assert_eq!(decisive.promoted().map(|r| r.confidence), Some(0.94));

        let weak = store
            .record_observation(
                Observation::new(
                    RuleCategory::Types,
                    "src/**/*.ts",
                    RuleSource::AstScan,
                    "Declare object shapes with interface rather than a type alias",
                )
                .with_confidence(0.61),
            )
            .unwrap();
        assert!(matches!(weak, Outcome::Created { .. }), "{weak:?}");
    }

    #[test]
    fn explicit_feedback_overrides_an_inferred_candidate_mid_flight() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        let instruction = "Use @AutoEntity instead of raw @Entity";

        store.record_observation(ast_diff(instruction)).unwrap();
        let outcome = store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                RuleSource::McpFeedback,
                instruction,
            ))
            .unwrap();

        let rule = outcome
            .promoted()
            .expect("an explicit confirmation should promote immediately");
        assert_eq!(rule.source, RuleSource::McpFeedback);
        assert_eq!(rule.occurrences, 2);
    }

    #[test]
    fn seeing_a_confirmed_rule_again_reinforces_it_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        let instruction = "Always return a standardized ApiResponse wrapper from controllers";

        store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.controller.ts",
                RuleSource::McpFeedback,
                instruction,
            ))
            .unwrap();

        let again = store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.controller.ts",
                RuleSource::AstDiff,
                instruction,
            ))
            .unwrap();

        match again {
            Outcome::AlreadyConfirmed {
                confidence,
                occurrences,
                ..
            } => {
                assert_eq!(occurrences, 2);
                assert_eq!(confidence, 0.96);
            }
            other => panic!("expected reinforcement of the confirmed rule, got {other:?}"),
        }
        assert_eq!(store.rules().unwrap().rules.len(), 1);
    }

    #[test]
    fn contradictions_decay_a_rule_and_eventually_demote_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let rule = store
            .record_observation(Observation::new(
                RuleCategory::ErrorHandling,
                "src/**/*.ts",
                RuleSource::McpFeedback,
                "Throw AppException rather than the built-in Error",
            ))
            .unwrap()
            .promoted()
            .unwrap()
            .clone();

        // 0.95 -> 0.71 -> 0.53, still confirmed but no longer active.
        assert!(store.apply_contradiction(&rule.id).unwrap());
        assert_eq!(
            store.rules().unwrap().find(&rule.id).unwrap().confidence,
            0.71
        );
        assert!(store.active_rules().unwrap().is_empty());

        assert!(store.apply_contradiction(&rule.id).unwrap());
        assert_eq!(
            store.rules().unwrap().find(&rule.id).unwrap().confidence,
            0.53
        );

        // 0.40 crosses the demotion line, so it goes back to being a candidate.
        assert!(store.apply_contradiction(&rule.id).unwrap());
        assert!(store.rules().unwrap().find(&rule.id).is_none());

        let demoted = store.candidates.get_candidate(&rule.id).unwrap().unwrap();
        assert_eq!(demoted.confidence, 0.40);
        assert_eq!(demoted.contradictions, 1);
    }

    #[test]
    fn contradicting_an_unknown_id_is_reported_rather_than_silently_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        assert!(!store.apply_contradiction("does-not-exist").unwrap());
    }

    #[test]
    fn disabling_a_rule_keeps_it_on_file_but_out_of_the_active_set() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let rule = store
            .record_observation(Observation::new(
                RuleCategory::Naming,
                "**/*",
                RuleSource::Manual,
                "Name boolean fields with an is or has prefix",
            ))
            .unwrap()
            .promoted()
            .unwrap()
            .clone();

        assert!(store.set_enabled(&rule.id, false).unwrap());
        assert_eq!(store.rules().unwrap().rules.len(), 1);
        assert!(store.active_rules().unwrap().is_empty());

        assert!(store.set_enabled(&rule.id, true).unwrap());
        assert_eq!(store.active_rules().unwrap().len(), 1);

        assert!(!store.set_enabled("unknown", false).unwrap());
    }

    #[test]
    fn the_opposite_of_a_confirmed_rule_is_refused_rather_than_installed_beside_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        let rule = store
            .record_observation(Observation::new(
                RuleCategory::Types,
                "**/*.{ts,tsx}",
                RuleSource::Manual,
                crate::ast::PREFER_INTERFACE,
            ))
            .unwrap()
            .promoted()
            .unwrap()
            .clone();

        let outcome = store
            .record_observation(Observation::new(
                RuleCategory::Types,
                "src/**/*.ts",
                RuleSource::AstDiff,
                crate::ast::PREFER_TYPE_ALIAS,
            ))
            .unwrap();
        match outcome {
            Outcome::Conflicts { conflicts_with, .. } => assert_eq!(conflicts_with, rule.id),
            other => panic!("expected a conflict, got {other:?}"),
        }
        assert_eq!(store.rules().unwrap().rules.len(), 1);
        assert!(store.candidate_rules().unwrap().is_empty());
    }

    #[test]
    fn an_unchanged_project_context_does_not_rewrite_the_tracked_file() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        let context = |files: usize| ProjectContext {
            detected_stack: vec!["react".to_string()],
            total_files_scanned: files,
            last_updated: Utc::now(),
        };
        store.set_project_context(context(10)).unwrap();
        let first = std::fs::read_to_string(&store.paths().rules_json).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        store.set_project_context(context(10)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&store.paths().rules_json).unwrap(),
            first
        );

        store.set_project_context(context(11)).unwrap();
        assert_ne!(
            std::fs::read_to_string(&store.paths().rules_json).unwrap(),
            first
        );
    }

    #[test]
    fn ensuring_the_rules_file_creates_it_once_and_never_resets_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());
        assert!(store.ensure_rules_file().unwrap());
        assert!(store.paths().rules_json.exists());

        store
            .record_observation(Observation::new(
                RuleCategory::Testing,
                "**/*.spec.ts",
                RuleSource::Manual,
                "Co-locate spec files beside the implementation",
            ))
            .unwrap();
        assert!(!store.ensure_rules_file().unwrap());
        assert_eq!(
            store.rules().unwrap().rules.len(),
            1,
            "an existing file is kept"
        );
    }

    #[test]
    fn deleting_clears_both_the_confirmed_rule_and_any_candidate_trace() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let rule = store
            .record_observation(Observation::new(
                RuleCategory::Testing,
                "**/*.spec.ts",
                RuleSource::Manual,
                "Co-locate spec files beside the implementation",
            ))
            .unwrap()
            .promoted()
            .unwrap()
            .clone();

        assert!(store.delete_rule(&rule.id).unwrap());
        assert!(store.rules().unwrap().rules.is_empty());
        assert!(store.candidate_rules().unwrap().is_empty());
        assert!(store.candidates().is_rejected(&rule.id).unwrap());
        assert!(!store.delete_rule(&rule.id).unwrap());
    }

    #[test]
    fn promoting_a_candidate_by_hand_moves_it_across_the_storage_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let id = store
            .record_observation(ast_diff("Use @AutoEntity instead of raw @Entity"))
            .unwrap()
            .id()
            .to_string();

        let promoted = store.promote_candidate(&id).unwrap().unwrap();
        assert_eq!(promoted.id, id);
        assert!(store.candidate_rules().unwrap().is_empty());
        assert_eq!(store.rules().unwrap().rules.len(), 1);

        assert!(store.promote_candidate("unknown").unwrap().is_none());
    }

    #[test]
    fn rules_for_file_returns_only_the_rules_scoped_to_that_path() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        for (category, scope, instruction) in [
            (
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                "Use @AutoEntity instead of raw @Entity",
            ),
            (
                RuleCategory::CodeStyle,
                "**/*",
                "Prefer early returns over nested conditionals",
            ),
            (
                RuleCategory::Testing,
                "**/*.spec.ts",
                "Co-locate spec files beside the implementation",
            ),
        ] {
            store
                .record_observation(Observation::new(
                    category,
                    scope,
                    RuleSource::Manual,
                    instruction,
                ))
                .unwrap();
        }

        let entity = store.rules_for_file("src/users/user.entity.ts").unwrap();
        let mut categories: Vec<_> = entity.iter().map(|r| r.category).collect();
        categories.sort();
        assert_eq!(
            categories,
            vec![RuleCategory::Architecture, RuleCategory::CodeStyle]
        );

        let spec = store.rules_for_file("src/users/user.spec.ts").unwrap();
        let mut spec_categories: Vec<_> = spec.iter().map(|r| r.category).collect();
        spec_categories.sort();
        assert_eq!(
            spec_categories,
            vec![RuleCategory::CodeStyle, RuleCategory::Testing]
        );
    }

    #[test]
    fn rules_for_file_accepts_an_absolute_path_inside_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                RuleSource::Manual,
                "Use @AutoEntity instead of raw @Entity",
            ))
            .unwrap();

        let absolute = tmp.path().join("src").join("users").join("user.entity.ts");
        assert_eq!(store.rules_for_file(absolute).unwrap().len(), 1);
    }

    #[test]
    fn disabled_rules_are_withheld_from_the_editor() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        let rule = store
            .record_observation(Observation::new(
                RuleCategory::Architecture,
                "src/**/*.entity.ts",
                RuleSource::Manual,
                "Use @AutoEntity instead of raw @Entity",
            ))
            .unwrap()
            .promoted()
            .unwrap()
            .clone();

        store.set_enabled(&rule.id, false).unwrap();
        assert!(store
            .rules_for_file("src/users/user.entity.ts")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_project_context_is_recorded_without_disturbing_the_rules() {
        let tmp = tempfile::tempdir().unwrap();
        let store = store(tmp.path());

        store
            .record_observation(ast_diff("Use @AutoEntity instead of raw @Entity"))
            .unwrap();

        store
            .set_project_context(ProjectContext {
                detected_stack: vec!["typescript".to_string(), "nestjs".to_string()],
                total_files_scanned: 342,
                last_updated: Utc::now(),
            })
            .unwrap();

        let loaded = store.rules().unwrap();
        assert_eq!(loaded.project_context.total_files_scanned, 342);
        assert_eq!(loaded.project_context.detected_stack.len(), 2);
    }
}
