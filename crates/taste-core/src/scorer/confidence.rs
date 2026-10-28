//! Confidence scoring, promotion and decay, per SPEC.md section 5.1.
//!
//! Every function rounds through [`round2`], so a confidence value never
//! differs between memory, SQLite and the git-tracked JSON file.

use crate::config::{DEMOTION_CONFIDENCE, PROMOTION_CONFIDENCE, PROMOTION_OCCURRENCES};
use crate::models::{round2, CandidateRule, RuleSource};

/// Starting confidence for a human correcting AI-generated code.
pub const INITIAL_AST_DIFF: f32 = 0.45;
/// Starting confidence for a preference the developer stated outright.
pub const INITIAL_EXPLICIT: f32 = 0.95;
/// Starting confidence for a cold-start scan whose ratio is unknown.
pub const INITIAL_AST_SCAN: f32 = 0.80;

/// Fraction of the remaining headroom gained per reinforcing observation.
pub const REINFORCEMENT_RATE: f32 = 0.20;
/// Ceiling confidence; a rule is never treated as certain.
pub const CONFIDENCE_CEILING: f32 = 0.99;
/// Multiplier applied when code contradicting a rule is committed.
pub const CONTRADICTION_FACTOR: f32 = 0.75;

/// Confidence a brand new candidate from `source` starts at.
///
/// Cold-start scans normally override this with the measured codebase ratio via
/// [`CandidateRule::with_confidence`].
pub fn initial_confidence(source: RuleSource) -> f32 {
    match source {
        RuleSource::AstDiff => INITIAL_AST_DIFF,
        RuleSource::McpFeedback | RuleSource::Manual => INITIAL_EXPLICIT,
        RuleSource::AstScan => INITIAL_AST_SCAN,
    }
}

/// Move confidence a fifth of the way toward certainty, capped at the ceiling.
///
/// `new = min(0.99, old + 0.20 * (1.0 - old))`
pub fn reinforce(confidence: f32) -> f32 {
    if confidence >= CONFIDENCE_CEILING {
        return CONFIDENCE_CEILING;
    }
    let raised = round2(confidence + REINFORCEMENT_RATE * (1.0 - confidence));
    // Close to the ceiling the raw gain is smaller than the rounding step, which
    // would stall the curve short of 0.99. Guarantee a minimum step so repeated
    // confirmation always counts for something.
    let stepped = if raised <= confidence {
        round2(confidence + 0.01)
    } else {
        raised
    };
    stepped.min(CONFIDENCE_CEILING)
}

/// Cut confidence by a quarter after the developer contradicted the rule.
///
/// `new = old * 0.75`
pub fn decay(confidence: f32) -> f32 {
    round2(confidence * CONTRADICTION_FACTOR)
}

/// Whether a candidate has earned a place in the git-tracked rule file.
///
/// The occurrence gate only applies to sources that report isolated events; see
/// [`RuleSource::bypasses_corroboration`].
pub fn is_promotable(candidate: &CandidateRule) -> bool {
    candidate.confidence >= PROMOTION_CONFIDENCE
        && (candidate.source.bypasses_corroboration()
            || candidate.occurrences >= PROMOTION_OCCURRENCES)
}

/// Whether a confirmed rule has decayed far enough to lose its promotion.
pub fn should_demote(confidence: f32) -> bool {
    confidence < DEMOTION_CONFIDENCE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleCategory;

    fn candidate(source: RuleSource, confidence: f32, occurrences: u32) -> CandidateRule {
        let mut c = CandidateRule::new(
            RuleCategory::Architecture,
            "src/**/*.ts",
            source,
            "Use the ApiResponse wrapper",
        );
        c.confidence = confidence;
        c.occurrences = occurrences;
        c
    }

    #[test]
    fn initial_confidence_matches_the_spec() {
        assert_eq!(initial_confidence(RuleSource::AstDiff), 0.45);
        assert_eq!(initial_confidence(RuleSource::McpFeedback), 0.95);
        assert_eq!(initial_confidence(RuleSource::Manual), 0.95);
        assert_eq!(initial_confidence(RuleSource::AstScan), 0.80);
    }

    #[test]
    fn reinforcement_follows_the_expected_curve_from_an_ast_diff_start() {
        // An ast-diff rule needs six total observations to clear 0.80.
        let mut confidence = initial_confidence(RuleSource::AstDiff);
        let mut curve = vec![confidence];
        for _ in 0..5 {
            confidence = reinforce(confidence);
            curve.push(confidence);
        }
        assert_eq!(curve, vec![0.45, 0.56, 0.65, 0.72, 0.78, 0.82]);
        assert!(curve[4] < PROMOTION_CONFIDENCE);
        assert!(curve[5] >= PROMOTION_CONFIDENCE);
    }

    #[test]
    fn reinforcement_never_exceeds_the_ceiling() {
        let mut confidence = 0.95;
        for _ in 0..50 {
            confidence = reinforce(confidence);
            assert!(confidence <= CONFIDENCE_CEILING);
        }
        assert_eq!(confidence, CONFIDENCE_CEILING);
    }

    #[test]
    fn reinforcement_is_monotonic() {
        let mut confidence = 0.1;
        for _ in 0..20 {
            let next = reinforce(confidence);
            assert!(next >= confidence, "{next} < {confidence}");
            confidence = next;
        }
    }

    #[test]
    fn decay_quarters_confidence_until_demotion() {
        let mut confidence = 0.95;
        assert!(!should_demote(confidence));

        confidence = decay(confidence); // 0.71
        assert_eq!(confidence, 0.71);
        assert!(!should_demote(confidence));

        confidence = decay(confidence); // 0.53
        assert_eq!(confidence, 0.53);
        assert!(!should_demote(confidence));

        confidence = decay(confidence); // 0.40
        assert_eq!(confidence, 0.40);
        assert!(should_demote(confidence));
    }

    #[test]
    fn demotion_boundary_sits_exactly_at_the_threshold() {
        assert!(should_demote(0.49));
        assert!(!should_demote(DEMOTION_CONFIDENCE));
        assert!(!should_demote(0.51));
    }

    #[test]
    fn promotion_requires_both_confidence_and_corroboration() {
        // High confidence but only two sightings.
        assert!(!is_promotable(&candidate(RuleSource::AstDiff, 0.82, 2)));
        // Three sightings but still uncertain.
        assert!(!is_promotable(&candidate(RuleSource::AstDiff, 0.78, 3)));
        // Both gates cleared.
        assert!(is_promotable(&candidate(RuleSource::AstDiff, 0.82, 3)));
        // Exactly on both thresholds.
        assert!(is_promotable(&candidate(
            RuleSource::AstDiff,
            PROMOTION_CONFIDENCE,
            PROMOTION_OCCURRENCES
        )));
    }

    #[test]
    fn explicit_feedback_promotes_on_its_first_sighting() {
        assert!(is_promotable(&candidate(RuleSource::McpFeedback, 0.95, 1)));
        assert!(is_promotable(&candidate(RuleSource::Manual, 0.95, 1)));
        // Even an explicit rule has to clear the confidence bar, which it only
        // fails after the developer has contradicted it.
        assert!(!is_promotable(&candidate(RuleSource::McpFeedback, 0.71, 1)));
    }

    #[test]
    fn a_decisive_cold_start_ratio_promotes_without_repeat_sightings() {
        // The scan already aggregated across the codebase, so one observation
        // carrying a high ratio is enough.
        assert!(is_promotable(&candidate(RuleSource::AstScan, 0.94, 1)));
        // An indecisive ratio still has to earn its place like anything else.
        assert!(!is_promotable(&candidate(RuleSource::AstScan, 0.61, 1)));
    }
}
