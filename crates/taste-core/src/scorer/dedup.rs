//! Recognising when a newly observed pattern is something already known.
//!
//! Without this, every rephrasing of the same preference would create a fresh
//! low-confidence candidate and nothing would ever accumulate enough
//! observations to be promoted.
//!
//! Two tests, either of which is enough:
//!
//! - **Wording.** The instructions share enough significant tokens
//!   ([`token_jaccard`]) and the scopes overlap.
//! - **Structure.** The scope is the same, and the avoid and prefer snippets
//!   have the same syntax shape ([`same_edit`]), however the instructions are
//!   phrased. More languages and an AI in chat mean more phrasings of one
//!   preference; the edit itself is the stable part.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use tree_sitter::Node;

use crate::ast::{parse, SupportedLanguage};
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

/// What a rule or observation says, for comparing one against another.
#[derive(Debug, Clone, Copy)]
pub struct Pattern<'a> {
    pub category: RuleCategory,
    pub scope: &'a str,
    pub instruction: &'a str,
    pub anti_pattern: Option<&'a str>,
    pub preferred_pattern: Option<&'a str>,
}

/// Whether two patterns are the same rule, by wording or by structure.
pub fn is_same_pattern(left: Pattern<'_>, right: Pattern<'_>) -> bool {
    if is_duplicate(
        left.category,
        left.scope,
        left.instruction,
        right.category,
        right.scope,
        right.instruction,
    ) {
        return true;
    }
    let (Some(left_anti), Some(left_pref), Some(right_anti), Some(right_pref)) = (
        left.anti_pattern,
        left.preferred_pattern,
        right.anti_pattern,
        right.preferred_pattern,
    ) else {
        return false;
    };
    left.category == right.category
        && left.scope.trim() == right.scope.trim()
        && same_edit(left.scope, (left_anti, left_pref), (right_anti, right_pref))
}

/// Whether two `(avoid, prefer)` snippet pairs make the same edit in the
/// language `scope` names.
///
/// The shape is the syntax tree with literal contents and comments dropped.
/// Identifiers that appear in both halves of a pair are what the edit carried
/// through, such as the variable in `console.log(user)` becoming
/// `logger.info(user)`; they are renamed by position. Every other identifier
/// is what the edit changed and is kept verbatim, so `@Entity` to
/// `@AutoEntity` never matches `@Index` to `@AutoIndex`. A scope that names no
/// single language, or a snippet that does not parse cleanly, never matches.
pub fn same_edit(scope: &str, left: (&str, &str), right: (&str, &str)) -> bool {
    let Some(lang) = scope_language(scope) else {
        return false;
    };
    match (edit_shape(lang, left), edit_shape(lang, right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

/// The language every file in `scope` is written in, read from its extension.
/// `**/*` and a brace list spanning languages name none.
fn scope_language(scope: &str) -> Option<SupportedLanguage> {
    let ext = scope.trim().rsplit_once('.')?.1;
    let exts: Vec<&str> = match ext.strip_prefix('{').and_then(|e| e.strip_suffix('}')) {
        Some(list) => list.split(',').collect(),
        None => vec![ext],
    };
    let mut langs = exts
        .iter()
        .map(|ext| SupportedLanguage::from_path(Path::new(&format!("x.{ext}"))));
    let first = langs.next()??;
    langs
        .all(|lang| lang.is_some_and(|lang| lang.family() == first.family()))
        .then_some(first)
}

fn edit_shape(lang: SupportedLanguage, (avoid, prefer): (&str, &str)) -> Option<String> {
    let avoid_ids = identifiers(lang, avoid)?;
    let prefer_ids = identifiers(lang, prefer)?;
    let carried: HashSet<&str> = avoid_ids
        .iter()
        .map(String::as_str)
        .filter(|id| prefer_ids.iter().any(|other| other == id))
        .collect();
    let mut renamed = HashMap::new();
    let mut shape = String::new();
    for snippet in [avoid, prefer] {
        let tree = parse(lang, snippet)?;
        shape_of(
            tree.root_node(),
            snippet,
            &carried,
            &mut renamed,
            &mut shape,
        );
        shape.push('|');
    }
    Some(shape)
}

fn identifiers(lang: SupportedLanguage, snippet: &str) -> Option<Vec<String>> {
    let tree = parse(lang, snippet)?;
    if tree.root_node().has_error() {
        return None;
    }
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_identifier(node.kind()) {
            found.push(snippet[node.byte_range()].to_string());
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    Some(found)
}

fn shape_of(
    node: Node<'_>,
    source: &str,
    carried: &HashSet<&str>,
    renamed: &mut HashMap<String, usize>,
    out: &mut String,
) {
    let kind = node.kind();
    if kind.contains("comment") {
        return;
    }
    if is_identifier(kind) {
        let text = &source[node.byte_range()];
        if carried.contains(text) {
            let next = renamed.len();
            let index = *renamed.entry(text.to_string()).or_insert(next);
            out.push_str(&format!(" ${index}"));
        } else {
            out.push(' ');
            out.push_str(text);
        }
        return;
    }
    if is_literal(kind) || node.child_count() == 0 {
        out.push(' ');
        out.push_str(kind);
        return;
    }
    out.push_str(" (");
    out.push_str(kind);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        shape_of(child, source, carried, renamed, out);
    }
    out.push(')');
}

fn is_identifier(kind: &str) -> bool {
    kind.ends_with("identifier")
}

/// Nodes whose contents are data, not structure: two edits that differ only
/// in a message or a number are the same edit.
fn is_literal(kind: &str) -> bool {
    matches!(
        kind,
        "string"
            | "template_string"
            | "string_literal"
            | "raw_string_literal"
            | "interpreted_string_literal"
            | "char_literal"
            | "rune_literal"
            | "number"
            | "integer"
            | "float"
            | "integer_literal"
            | "float_literal"
            | "int_literal"
            | "imaginary_literal"
    )
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

    fn pattern<'a>(
        scope: &'a str,
        instruction: &'a str,
        anti: &'a str,
        preferred: &'a str,
    ) -> Pattern<'a> {
        Pattern {
            category: RuleCategory::CodeStyle,
            scope,
            instruction,
            anti_pattern: Some(anti),
            preferred_pattern: Some(preferred),
        }
    }

    #[test]
    fn the_same_edit_in_different_words_is_the_same_rule() {
        let detected = pattern(
            "src/**/*.ts",
            "Prefer a logger service over `console.log` and `console.error`.",
            "console.log(user)",
            "logger.info(user)",
        );
        let from_chat = pattern(
            "src/**/*.ts",
            "Write to the logger, never to the console.",
            "console.log(order, total)",
            "logger.info(order, total)",
        );
        assert!(token_jaccard(detected.instruction, from_chat.instruction) < DUPLICATE_JACCARD);
        assert!(
            !same_edit(
                "src/**/*.ts",
                ("console.log(user)", "logger.info(user)"),
                ("console.log(order, total)", "logger.info(order, total)"),
            ),
            "a different argument count is a different shape"
        );
        let from_chat = pattern(
            "src/**/*.ts",
            "Write to the logger, never to the console.",
            "console.log(order)",
            "logger.info(order)",
        );
        assert!(is_same_pattern(detected, from_chat));
    }

    #[test]
    fn a_different_message_or_number_is_still_the_same_edit() {
        assert!(same_edit(
            "**/*.py",
            (
                "raise Exception(\"missing\", 404)",
                "raise NotFound(\"missing\", 404)"
            ),
            (
                "raise Exception(\"gone\", 410)",
                "raise NotFound(\"gone\", 410)"
            ),
        ));
    }

    #[test]
    fn the_same_words_with_different_snippets_are_not_merged_by_structure() {
        let entity = pattern(
            "src/**/*.ts",
            "Use the project decorator.",
            "@Entity()\nclass A {}",
            "@AutoEntity()\nclass A {}",
        );
        let index = pattern(
            "src/**/*.ts",
            "Prefer project indexes everywhere.",
            "@Index()\nclass A {}",
            "@AutoIndex()\nclass A {}",
        );
        assert!(!is_same_pattern(entity, index));
    }

    #[test]
    fn snippets_in_two_languages_never_merge() {
        let python = pattern(
            "**/*.py",
            "Log, do not print.",
            "print(user)",
            "logger.info(user)",
        );
        let script = pattern(
            "**/*.ts",
            "Use the logger.",
            "print(user)",
            "logger.info(user)",
        );
        assert!(!is_same_pattern(python, script), "the scopes differ");
        assert!(!same_edit(
            "**/*",
            ("print(user)", "logger.info(user)"),
            ("print(user)", "logger.info(user)")
        ));
        assert!(!same_edit(
            "**/*.{ts,py}",
            ("print(user)", "logger.info(user)"),
            ("print(user)", "logger.info(user)")
        ));
    }

    #[test]
    fn a_snippet_that_does_not_parse_never_matches() {
        assert!(!same_edit(
            "**/*.ts",
            ("console.log(", "logger.info("),
            ("console.log(", "logger.info(")
        ));
    }
}
