//! Rule shapes shared by storage, scoring, the transpiler and the MCP server.
//!
//! Two invariants matter here and are enforced rather than documented:
//!
//! - Confidence is always a 2-decimal value, both in memory and on disk, so the
//!   git-tracked rule file never churns on float formatting noise.
//! - Rule identifiers are derived from the category and instruction text, so
//!   re-observing the same preference reinforces one rule instead of piling up
//!   near-duplicates.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{SCHEMA_URL, SCHEMA_VERSION};
use crate::error::Error;

/// What aspect of a codebase a rule governs.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum RuleCategory {
    Architecture,
    CodeStyle,
    Imports,
    Types,
    ErrorHandling,
    Naming,
    Testing,
}

impl RuleCategory {
    /// Every category, in the order they appear in generated documents.
    pub const ALL: [RuleCategory; 7] = [
        RuleCategory::Architecture,
        RuleCategory::CodeStyle,
        RuleCategory::Imports,
        RuleCategory::Types,
        RuleCategory::ErrorHandling,
        RuleCategory::Naming,
        RuleCategory::Testing,
    ];

    /// Wire and filename form, e.g. `error-handling`.
    pub fn slug(self) -> &'static str {
        match self {
            RuleCategory::Architecture => "architecture",
            RuleCategory::CodeStyle => "code-style",
            RuleCategory::Imports => "imports",
            RuleCategory::Types => "types",
            RuleCategory::ErrorHandling => "error-handling",
            RuleCategory::Naming => "naming",
            RuleCategory::Testing => "testing",
        }
    }

    /// Short prefix used when building rule identifiers.
    pub fn id_prefix(self) -> &'static str {
        match self {
            RuleCategory::Architecture => "arch",
            RuleCategory::CodeStyle => "style",
            RuleCategory::Imports => "imports",
            RuleCategory::Types => "types",
            RuleCategory::ErrorHandling => "errors",
            RuleCategory::Naming => "naming",
            RuleCategory::Testing => "testing",
        }
    }

    /// Human-readable heading, e.g. `Error Handling`.
    pub fn title(self) -> &'static str {
        match self {
            RuleCategory::Architecture => "Architecture",
            RuleCategory::CodeStyle => "Code Style",
            RuleCategory::Imports => "Imports",
            RuleCategory::Types => "Types",
            RuleCategory::ErrorHandling => "Error Handling",
            RuleCategory::Naming => "Naming",
            RuleCategory::Testing => "Testing",
        }
    }
}

impl fmt::Display for RuleCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for RuleCategory {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RuleCategory::ALL
            .into_iter()
            .find(|c| c.slug() == s)
            .ok_or_else(|| Error::InvalidEnumValue {
                kind: "rule category",
                value: s.to_string(),
            })
    }
}

/// Which intelligence loop produced a rule.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSource {
    AstScan,
    AstDiff,
    McpFeedback,
    Manual,
}

impl RuleSource {
    const ALL: [RuleSource; 4] = [
        RuleSource::AstScan,
        RuleSource::AstDiff,
        RuleSource::McpFeedback,
        RuleSource::Manual,
    ];

    /// Wire form, e.g. `ast-diff`.
    pub fn slug(self) -> &'static str {
        match self {
            RuleSource::AstScan => "ast-scan",
            RuleSource::AstDiff => "ast-diff",
            RuleSource::McpFeedback => "mcp-feedback",
            RuleSource::Manual => "manual",
        }
    }

    /// Whether this source reflects a direct, explicit statement from the
    /// developer rather than something OpenTaste inferred.
    pub fn is_explicit(self) -> bool {
        matches!(self, RuleSource::McpFeedback | RuleSource::Manual)
    }

    /// Whether an observation from this source already carries its own
    /// corroboration and so need not wait for repeat sightings.
    ///
    /// `AstDiff` is the only source that reports one isolated event. A cold
    /// start scan has already measured a ratio across the whole codebase before
    /// it emits anything, and explicit feedback is the developer stating the
    /// rule outright, so neither has anything left to corroborate.
    pub fn bypasses_corroboration(self) -> bool {
        !matches!(self, RuleSource::AstDiff)
    }
}

impl fmt::Display for RuleSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

impl FromStr for RuleSource {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RuleSource::ALL
            .into_iter()
            .find(|c| c.slug() == s)
            .ok_or_else(|| Error::InvalidEnumValue {
                kind: "rule source",
                value: s.to_string(),
            })
    }
}

/// Clamp to `0.0..=1.0` and round to two decimals.
///
/// Every confidence mutation funnels through here so the in-memory value always
/// equals the value that will be written to disk.
pub fn round2(confidence: f32) -> f32 {
    let clamped = confidence.clamp(0.0, 1.0);
    (clamped * 100.0).round() / 100.0
}

/// Serde adapter keeping confidence at two decimals in both directions.
mod confidence_serde {
    use super::round2;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f32, serializer: S) -> Result<S::Ok, S::Error> {
        // Round through integer hundredths and emit an f64 so the JSON writer
        // prints `0.95` rather than the nearest f32 expansion of it.
        let hundredths = (value.clamp(0.0, 1.0) * 100.0).round() as i64;
        serializer.serialize_f64(hundredths as f64 / 100.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
        Ok(round2(f32::deserialize(deserializer)?))
    }
}

/// A confirmed rule, stored in the git-tracked `.taste/rules.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TasteRule {
    pub id: String,
    pub category: RuleCategory,
    /// Glob pattern the rule applies to, e.g. `src/**/*.ts` or `**/*`.
    pub scope: String,
    #[serde(with = "confidence_serde")]
    pub confidence: f32,
    pub occurrences: u32,
    pub enabled: bool,
    pub source: RuleSource,
    pub instruction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anti_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_pattern: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl TasteRule {
    /// Whether this rule should be compiled into editor instruction files.
    pub fn is_active(&self) -> bool {
        self.enabled && self.confidence >= crate::config::PROMOTION_CONFIDENCE
    }

    /// Demote back to a candidate, preserving counters.
    pub fn into_candidate(self) -> CandidateRule {
        CandidateRule {
            id: self.id,
            category: self.category,
            scope: self.scope,
            confidence: self.confidence,
            occurrences: self.occurrences,
            contradictions: 0,
            source: self.source,
            instruction: self.instruction,
            anti_pattern: self.anti_pattern,
            preferred_pattern: self.preferred_pattern,
            updated_at: self.updated_at,
        }
    }
}

/// An unpromoted pattern, stored in the local SQLite cache and never committed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CandidateRule {
    pub id: String,
    pub category: RuleCategory,
    pub scope: String,
    #[serde(with = "confidence_serde")]
    pub confidence: f32,
    pub occurrences: u32,
    pub contradictions: u32,
    pub source: RuleSource,
    pub instruction: String,
    pub anti_pattern: Option<String>,
    pub preferred_pattern: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl CandidateRule {
    /// Build a fresh candidate, deriving its identifier and initial confidence.
    pub fn new(
        category: RuleCategory,
        scope: impl Into<String>,
        source: RuleSource,
        instruction: impl Into<String>,
    ) -> Self {
        let instruction = instruction.into();
        let scope = scope.into();
        CandidateRule {
            id: rule_id(category, &scope, &instruction),
            category,
            scope,
            confidence: crate::scorer::confidence::initial_confidence(source),
            occurrences: 1,
            contradictions: 0,
            source,
            instruction,
            anti_pattern: None,
            preferred_pattern: None,
            updated_at: Utc::now(),
        }
    }

    /// Attach the example snippets that illustrate the rule.
    pub fn with_patterns(
        mut self,
        anti_pattern: Option<String>,
        preferred_pattern: Option<String>,
    ) -> Self {
        self.anti_pattern = anti_pattern;
        self.preferred_pattern = preferred_pattern;
        self
    }

    /// Override the starting confidence, used by the cold-start scanner which
    /// derives it from a measured codebase ratio.
    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = round2(confidence);
        self
    }

    /// Promote to a confirmed rule, enabled by default.
    pub fn into_rule(self) -> TasteRule {
        TasteRule {
            id: self.id,
            category: self.category,
            scope: self.scope,
            confidence: self.confidence,
            occurrences: self.occurrences,
            enabled: true,
            source: self.source,
            instruction: self.instruction,
            anti_pattern: self.anti_pattern,
            preferred_pattern: self.preferred_pattern,
            updated_at: self.updated_at,
        }
    }
}

/// What the scanner learned about the project as a whole.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectContext {
    pub detected_stack: Vec<String>,
    pub total_files_scanned: usize,
    pub last_updated: DateTime<Utc>,
}

impl Default for ProjectContext {
    fn default() -> Self {
        ProjectContext {
            detected_stack: Vec::new(),
            total_files_scanned: 0,
            last_updated: DateTime::UNIX_EPOCH,
        }
    }
}

/// Full contents of `.taste/rules.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RulesFile {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub version: String,
    pub project_context: ProjectContext,
    pub rules: Vec<TasteRule>,
}

impl Default for RulesFile {
    fn default() -> Self {
        RulesFile {
            schema: SCHEMA_URL.to_string(),
            version: SCHEMA_VERSION.to_string(),
            project_context: ProjectContext::default(),
            rules: Vec::new(),
        }
    }
}

impl RulesFile {
    /// An otherwise-default rule file holding `rules`, in canonical order.
    pub fn with_rules(rules: impl IntoIterator<Item = TasteRule>) -> Self {
        let mut file = RulesFile {
            rules: rules.into_iter().collect(),
            ..Default::default()
        };
        file.sort();
        file
    }

    /// Locate a rule by identifier.
    pub fn find(&self, id: &str) -> Option<&TasteRule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Locate a rule by identifier for mutation.
    pub fn find_mut(&mut self, id: &str) -> Option<&mut TasteRule> {
        self.rules.iter_mut().find(|r| r.id == id)
    }

    /// Insert a rule, replacing any existing one with the same identifier.
    pub fn upsert(&mut self, rule: TasteRule) {
        match self.rules.iter_mut().find(|r| r.id == rule.id) {
            Some(existing) => *existing = rule,
            None => self.rules.push(rule),
        }
        self.sort();
    }

    /// Remove a rule, returning it if it was present.
    pub fn remove(&mut self, id: &str) -> Option<TasteRule> {
        let index = self.rules.iter().position(|r| r.id == id)?;
        Some(self.rules.remove(index))
    }

    /// Order rules by identifier, the on-disk canonical order.
    pub fn sort(&mut self) {
        self.rules.sort_by(|a, b| a.id.cmp(&b.id));
    }

    /// Rules eligible for compilation into editor instruction files.
    pub fn active_rules(&self) -> Vec<&TasteRule> {
        self.rules.iter().filter(|r| r.is_active()).collect()
    }
}

/// Words carrying no distinguishing signal in an instruction sentence.
const STOPWORDS: &[&str] = &[
    "a", "all", "always", "an", "and", "any", "are", "as", "at", "be", "but", "by", "can", "dont",
    "every", "for", "from", "has", "have", "in", "instead", "into", "is", "it", "its", "must",
    "never", "not", "of", "on", "only", "or", "over", "prefer", "rather", "should", "than", "that",
    "the", "their", "them", "then", "they", "this", "to", "use", "using", "via", "when", "with",
    "within",
];

/// Collapse an instruction to a canonical form for identity and similarity
/// comparisons: lowercase, single-spaced, no trailing punctuation.
pub fn normalize_instruction(instruction: &str) -> String {
    let lowered = instruction.to_lowercase();
    let collapsed = lowered.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_end_matches(['.', '!', '?', ',', ';', ':'])
        .to_string()
}

/// Significant, order-preserving, de-duplicated tokens of an instruction.
pub fn significant_tokens(instruction: &str) -> Vec<String> {
    let normalized = normalize_instruction(instruction);
    let mut seen: Vec<String> = Vec::new();
    for raw in normalized.split(|c: char| !c.is_ascii_alphanumeric()) {
        if raw.len() < 3 || STOPWORDS.contains(&raw) {
            continue;
        }
        let token = raw.to_string();
        if !seen.contains(&token) {
            seen.push(token);
        }
    }
    seen
}

/// Number of instruction tokens used in the readable part of an identifier.
const ID_SLUG_TOKENS: usize = 5;

/// Derive a stable identifier from a rule's category, scope and instruction.
///
/// The readable slug keeps identifiers reviewable in git, while the hash suffix
/// guarantees two genuinely different rules in one category can never collapse
/// into each other just because they open with the same words. The scope is in
/// the hash because the same preference learned in two monorepo packages, or in
/// two languages, is two rules: one package's corrections must not count
/// towards another's.
pub fn rule_id(category: RuleCategory, scope: &str, instruction: &str) -> String {
    let tokens = significant_tokens(instruction);
    let slug = tokens
        .iter()
        .take(ID_SLUG_TOKENS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("-");

    let mut hasher = Sha256::new();
    hasher.update(category.slug().as_bytes());
    hasher.update(b"\0");
    hasher.update(scope.trim().as_bytes());
    hasher.update(b"\0");
    hasher.update(normalize_instruction(instruction).as_bytes());
    let digest = hasher.finalize();
    let suffix: String = digest
        .iter()
        .take(3)
        .map(|byte| format!("{byte:02x}"))
        .collect();

    if slug.is_empty() {
        format!("{}-{}", category.id_prefix(), suffix)
    } else {
        format!("{}-{}-{}", category.id_prefix(), slug, suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const SCOPE: &str = "src/**/*.ts";

    fn sample_rule() -> TasteRule {
        TasteRule {
            id: "arch-test-abc123".to_string(),
            category: RuleCategory::Architecture,
            scope: "src/**/*.entity.ts".to_string(),
            confidence: 0.95,
            occurrences: 14,
            enabled: true,
            source: RuleSource::AstDiff,
            instruction: "Use custom @AutoEntity() decorators.".to_string(),
            anti_pattern: Some("@Entity('users')".to_string()),
            preferred_pattern: Some("@AutoEntity()".to_string()),
            updated_at: DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[test]
    fn categories_and_sources_round_trip_through_strings() {
        for category in RuleCategory::ALL {
            assert_eq!(category.slug().parse::<RuleCategory>().unwrap(), category);
        }
        for source in RuleSource::ALL {
            assert_eq!(source.slug().parse::<RuleSource>().unwrap(), source);
        }
        assert!("not-a-category".parse::<RuleCategory>().is_err());
    }

    #[test]
    fn serde_uses_the_kebab_case_wire_format_from_the_spec() {
        let json = serde_json::to_string(&sample_rule()).unwrap();
        assert!(json.contains(r#""category":"architecture""#), "{json}");
        assert!(json.contains(r#""source":"ast-diff""#), "{json}");

        let code_style = serde_json::to_string(&RuleCategory::CodeStyle).unwrap();
        assert_eq!(code_style, r#""code-style""#);
    }

    #[test]
    fn confidence_serializes_without_float_noise() {
        for (value, expected) in [
            (0.95_f32, "0.95"),
            (0.8_f32, "0.8"),
            (0.45_f32, "0.45"),
            (0.648_f32, "0.65"),
            (1.0_f32, "1.0"),
            (0.0_f32, "0.0"),
        ] {
            let mut rule = sample_rule();
            rule.confidence = value;
            let json = serde_json::to_string(&rule).unwrap();
            assert!(
                json.contains(&format!(r#""confidence":{expected}"#)),
                "expected confidence {expected} in {json}"
            );
        }
    }

    #[test]
    fn confidence_survives_a_json_round_trip_bit_for_bit() {
        let rule = sample_rule();
        let json = serde_json::to_string(&rule).unwrap();
        let back: TasteRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn round2_clamps_and_rounds() {
        assert_eq!(round2(0.648), 0.65);
        assert_eq!(round2(1.4), 1.0);
        assert_eq!(round2(-0.2), 0.0);
        assert_eq!(round2(0.805), 0.81);
    }

    #[test]
    fn rule_ids_are_stable_and_instruction_sensitive() {
        let instruction = "Use custom @AutoEntity() and @AutoIndex() decorators instead of raw TypeORM @Entity() and @Index().";
        let id = rule_id(RuleCategory::Architecture, SCOPE, instruction);

        assert_eq!(id, rule_id(RuleCategory::Architecture, SCOPE, instruction));
        assert!(id.starts_with("arch-"), "{id}");
        assert!(id.contains("custom"), "{id}");
        assert!(id.contains("autoentity"), "{id}");

        // Same words, different category, and same category, different words.
        assert_ne!(id, rule_id(RuleCategory::Types, SCOPE, instruction));
        assert_ne!(
            id,
            rule_id(RuleCategory::Architecture, SCOPE, "Use custom loggers.")
        );
    }

    #[test]
    fn the_same_preference_in_two_packages_gets_two_ids() {
        let instruction = "Prefer custom error classes over throwing bare `Error`.";
        let nextjs = rule_id(
            RuleCategory::ErrorHandling,
            "apps/nextjs/**/*.tsx",
            instruction,
        );
        let start = rule_id(
            RuleCategory::ErrorHandling,
            "apps/tanstack-start/**/*.tsx",
            instruction,
        );
        assert_ne!(nextjs, start);
        // Only the hash differs, so both stay readable in a diff.
        assert_eq!(
            nextjs.rsplit_once('-').unwrap().0,
            start.rsplit_once('-').unwrap().0
        );
    }

    #[test]
    fn rule_ids_ignore_casing_whitespace_and_trailing_punctuation() {
        let a = rule_id(
            RuleCategory::Imports,
            SCOPE,
            "Prefer alias imports over relative.",
        );
        let b = rule_id(
            RuleCategory::Imports,
            SCOPE,
            "  prefer  ALIAS imports over relative  ",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn rule_ids_stay_readable_when_every_word_is_a_stopword() {
        let id = rule_id(RuleCategory::Naming, SCOPE, "Use the of to and");
        assert!(id.starts_with("naming-"), "{id}");
        assert_eq!(id.matches('-').count(), 1, "{id}");
    }

    #[test]
    fn significant_tokens_drops_noise_and_duplicates() {
        let tokens = significant_tokens("Always use the ApiResponse wrapper, the ApiResponse type");
        assert_eq!(tokens, vec!["apiresponse", "wrapper", "type"]);
    }

    #[test]
    fn rules_file_upsert_replaces_and_keeps_sorted_order() {
        let mut file = RulesFile::default();
        let mut first = sample_rule();
        first.id = "b-rule".to_string();
        let mut second = sample_rule();
        second.id = "a-rule".to_string();

        file.upsert(first.clone());
        file.upsert(second);
        assert_eq!(
            file.rules.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["a-rule", "b-rule"]
        );

        let mut updated = first;
        updated.occurrences = 99;
        file.upsert(updated);
        assert_eq!(file.rules.len(), 2);
        assert_eq!(file.find("b-rule").unwrap().occurrences, 99);
    }

    #[test]
    fn active_rules_filters_disabled_and_low_confidence() {
        let mut file = RulesFile::default();

        let mut active = sample_rule();
        active.id = "active".to_string();

        let mut disabled = sample_rule();
        disabled.id = "disabled".to_string();
        disabled.enabled = false;

        let mut weak = sample_rule();
        weak.id = "weak".to_string();
        weak.confidence = 0.79;

        file.upsert(active);
        file.upsert(disabled);
        file.upsert(weak);

        let ids: Vec<_> = file.active_rules().iter().map(|r| r.id.clone()).collect();
        assert_eq!(ids, vec!["active"]);
    }

    #[test]
    fn candidate_promotion_preserves_counters_and_enables_the_rule() {
        let candidate = CandidateRule::new(
            RuleCategory::Imports,
            "src/**/*.ts",
            RuleSource::AstDiff,
            "Prefer @/ alias imports over deep relative paths",
        );
        assert_eq!(candidate.confidence, 0.45);
        assert_eq!(candidate.occurrences, 1);

        let rule = candidate.clone().into_rule();
        assert_eq!(rule.id, candidate.id);
        assert_eq!(rule.occurrences, candidate.occurrences);
        assert!(rule.enabled);

        let back = rule.into_candidate();
        assert_eq!(back.id, candidate.id);
        assert_eq!(back.contradictions, 0);
    }
}
