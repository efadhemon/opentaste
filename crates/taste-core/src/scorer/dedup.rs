//! Recognising when a newly observed pattern is something already known.
//!
//! Without this, every rephrasing of the same preference would create a fresh
//! low-confidence candidate and nothing would ever accumulate enough
//! observations to be promoted.

use std::collections::BTreeSet;

use crate::models::{normalize_instruction, significant_tokens, RuleCategory};

/// Token overlap above which two instructions are considered the same rule.
pub const DUPLICATE_JACCARD: f32 = 0.85;

/// Glob that matches every file in the project.
const MATCH_ALL: &str = "**/*";

/// Jaccard similarity of the significant tokens of two instructions.
///
/// Returns `1.0` for two instructions that normalise to the same text, even if
/// neither contains a single significant token.
pub fn token_jaccard(left: &str, right: &str) -> f32 {
    if normalize_instruction(left) == normalize_instruction(right) {
        return 1.0;
    }

    let a: BTreeSet<String> = significant_tokens(left).into_iter().collect();
    let b: BTreeSet<String> = significant_tokens(right).into_iter().collect();

    if a.is_empty() && b.is_empty() {
        return 0.0;
    }

    let intersection = a.intersection(&b).count() as f32;
    let union = a.union(&b).count() as f32;
    intersection / union
}

/// Whether two scope globs address an overlapping set of files.
///
/// This is deliberately conservative: it recognises identical globs and the
/// project-wide `**/*`, and otherwise assumes the scopes are distinct. Treating
/// unrelated scopes as overlapping would silently merge rules that should stay
/// separate, which is the more damaging mistake.
pub fn scope_overlap(left: &str, right: &str) -> bool {
    let left = left.trim();
    let right = right.trim();
    left == right || left == MATCH_ALL || right == MATCH_ALL
}

/// Whether a newly observed pattern is the same rule as an existing one.
pub fn is_duplicate(
    left_category: RuleCategory,
    left_scope: &str,
    left_instruction: &str,
    right_category: RuleCategory,
    right_scope: &str,
    right_instruction: &str,
) -> bool {
    left_category == right_category
        && scope_overlap(left_scope, right_scope)
        && token_jaccard(left_instruction, right_instruction) >= DUPLICATE_JACCARD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_instructions_score_one_regardless_of_formatting() {
        assert_eq!(
            token_jaccard(
                "Use the ApiResponse wrapper.",
                "  use   THE ApiResponse wrapper  "
            ),
            1.0
        );
    }

    #[test]
    fn unrelated_instructions_score_near_zero() {
        let score = token_jaccard(
            "Use custom @AutoEntity decorators",
            "Prefer early returns over nested conditionals",
        );
        assert!(score < 0.2, "unexpectedly similar: {score}");
    }

    #[test]
    fn a_rephrasing_still_counts_as_the_same_rule() {
        let score = token_jaccard(
            "Always use the ApiResponse wrapper in controllers",
            "Use the ApiResponse wrapper in controllers",
        );
        assert_eq!(score, 1.0, "stopwords should not change identity");
    }

    #[test]
    fn instructions_made_only_of_stopwords_never_match_each_other() {
        assert_eq!(token_jaccard("use the", "of the and"), 0.0);
    }

    #[test]
    fn scope_overlap_recognises_identical_and_project_wide_globs() {
        assert!(scope_overlap("src/**/*.ts", "src/**/*.ts"));
        assert!(scope_overlap("**/*", "src/**/*.ts"));
        assert!(scope_overlap("src/**/*.ts", "**/*"));
        assert!(!scope_overlap("src/**/*.ts", "src/**/*.py"));
    }

    #[test]
    fn duplicates_need_category_scope_and_wording_to_agree() {
        let instruction = "Use the ApiResponse wrapper in controllers";
        let rephrased = "Always use the ApiResponse wrapper in controllers";

        assert!(is_duplicate(
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            instruction,
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            rephrased,
        ));

        // Same words, different category.
        assert!(!is_duplicate(
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            instruction,
            RuleCategory::Types,
            "src/**/*.controller.ts",
            instruction,
        ));

        // Same words, disjoint scope.
        assert!(!is_duplicate(
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            instruction,
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            instruction,
        ));

        // Same category and scope, unrelated wording.
        assert!(!is_duplicate(
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            instruction,
            RuleCategory::Architecture,
            "src/**/*.controller.ts",
            "Prefer early returns over nested conditionals",
        ));
    }
}
