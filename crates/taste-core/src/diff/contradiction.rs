//! Find active rules whose rejected snippet is still in a file.
//!
//! A false contradiction decays a correct rule, which is harder to notice than
//! a missing one. The match has to be a tight syntax node, not a mention in a
//! comment or a string, unless the file cannot be parsed — and then only a
//! long snippet is trusted as a normalised-whitespace substring.

use std::path::Path;

use tree_sitter::Node;

use crate::ast::{node_text, parse, SupportedLanguage};
use crate::error::Result;
use crate::storage::RuleStore;

const MIN_NEEDLE: usize = 8;
const FALLBACK_NEEDLE: usize = 24;
const SLACK: usize = 48;

/// Identifiers of active rules whose `anti_pattern` appears in `after`.
pub fn contradictions(store: &RuleStore, path: &Path, after: &str) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for rule in store.rules_for_file(path)? {
        let Some(anti) = rule.anti_pattern.as_deref() else {
            continue;
        };
        if anti_pattern_present(anti, after, path) {
            ids.push(rule.id);
        }
    }
    Ok(ids)
}

fn anti_pattern_present(anti: &str, after: &str, path: &Path) -> bool {
    let needle = normalize_ws(anti);
    if needle.len() < MIN_NEEDLE {
        return false;
    }
    let Some(lang) = SupportedLanguage::from_path(path) else {
        return needle.len() >= FALLBACK_NEEDLE && normalize_ws(after).contains(&needle);
    };
    let Some(tree) = parse(lang, after) else {
        return needle.len() >= FALLBACK_NEEDLE && normalize_ws(after).contains(&needle);
    };
    let mut found = false;
    visit(tree.root_node(), after, &needle, &mut found);
    found
}

fn visit(node: Node<'_>, source: &str, needle: &str, found: &mut bool) {
    if *found || is_skipped(node.kind()) {
        return;
    }
    let text = normalize_ws(node_text(node, source));
    if text.contains(needle)
        && text.len() <= needle.len() + SLACK
        && !child_holds_needle(node, source, needle)
    {
        *found = true;
        return;
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, source, needle, found);
    }
}

fn child_holds_needle(node: Node<'_>, source: &str, needle: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .any(|child| normalize_ws(node_text(child, source)).contains(needle));
    found
}

fn is_skipped(kind: &str) -> bool {
    matches!(
        kind,
        "comment"
            | "line_comment"
            | "block_comment"
            | "string"
            | "string_fragment"
            | "template_string"
            | "string_literal"
            | "string_content"
            | "interpreted_string_literal"
            | "raw_string_literal"
    )
}

fn normalize_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use crate::models::{RuleCategory, RuleSource};
    use crate::storage::{Observation, RuleStore};

    fn store_with(anti: &str, scope: &str) -> (tempfile::TempDir, RuleStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = RuleStore::open_ephemeral(Paths::for_root(dir.path())).unwrap();
        store
            .record_observation(
                Observation::new(
                    RuleCategory::Architecture,
                    scope,
                    RuleSource::McpFeedback,
                    "Use @AutoEntity instead of @Entity",
                )
                .with_patterns(Some(anti.to_string()), Some("@AutoEntity()".to_string())),
            )
            .unwrap();
        (dir, store)
    }

    #[test]
    fn a_decorator_still_in_the_file_contradicts_the_rule() {
        let (_dir, store) = store_with("@Entity(\"users\")", "src/**/*.ts");
        let ids = contradictions(
            &store,
            Path::new("src/users/user.ts"),
            "@Entity(\"users\")\nexport class User {}\n",
        )
        .unwrap();
        assert_eq!(ids.len(), 1, "{ids:?}");
    }

    #[test]
    fn the_same_text_inside_a_comment_does_not_contradict() {
        let (_dir, store) = store_with("@Entity(\"users\")", "src/**/*.ts");
        let ids = contradictions(
            &store,
            Path::new("src/users/user.ts"),
            "// keep @Entity(\"users\") out of this file\nexport class User {}\n",
        )
        .unwrap();
        assert!(ids.is_empty(), "{ids:?}");
    }

    #[test]
    fn whitespace_differences_still_match_a_throw() {
        let (_dir, store) = store_with("throw new Error(\"missing\")", "src/**/*.ts");
        let ids = contradictions(
            &store,
            Path::new("src/app.ts"),
            "function load() {\n  throw   new   Error(\"missing\");\n}\n",
        )
        .unwrap();
        assert_eq!(ids.len(), 1, "{ids:?}");
    }

    #[test]
    fn a_file_outside_the_rule_scope_is_not_a_contradiction() {
        let (_dir, store) = store_with("@Entity(\"users\")", "src/**/*.entity.ts");
        let ids = contradictions(
            &store,
            Path::new("src/users/user.service.ts"),
            "@Entity(\"users\")\nexport class User {}\n",
        )
        .unwrap();
        assert!(ids.is_empty(), "{ids:?}");
    }
}
