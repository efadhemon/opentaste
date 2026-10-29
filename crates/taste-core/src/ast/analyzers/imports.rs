//! Import paths and export shape.

use tree_sitter::{Node, Tree};

use super::{run, Count};
use crate::ast::query::{self, ScriptQuery};
use crate::ast::SupportedLanguage;

static IMPORTS: ScriptQuery = ScriptQuery::new("(import_statement source: (string) @source) @stmt");

static EXPORTS: ScriptQuery = ScriptQuery::new("(export_statement) @export");

pub(super) fn import_paths(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut alias = 0u32;
    let mut deep = 0u32;
    for (index, node) in run(&IMPORTS, lang, tree, source) {
        if query_name(&IMPORTS, lang, index) != "source" {
            continue;
        }
        let path = query::unquote(query::node_text(node, source));
        if is_alias(path) {
            alias += 1;
        } else if is_deep_relative(path) {
            deep += 1;
        }
    }
    Count::from_sides(alias, deep)
}

pub(super) fn exports(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut named = 0u32;
    let mut defaulted = 0u32;
    for (_, node) in run(&EXPORTS, lang, tree, source) {
        // Re-exports (`export * from`, `export { x } from`) and type-only
        // lists (`export type { A }`) say how a barrel is assembled, not how
        // modules export what they define. Counting them made barrel-heavy
        // projects look more "named" than their own code is.
        if node.child_by_field_name("source").is_some() || is_type_only_list(node) {
            continue;
        }
        if has_anonymous(node, "default") {
            defaulted += 1;
        } else {
            named += 1;
        }
    }
    Count::from_sides(named, defaulted)
}

fn query_name(query: &ScriptQuery, lang: SupportedLanguage, index: usize) -> &str {
    query
        .get(lang)
        .and_then(|compiled| compiled.capture_names().get(index).copied())
        .unwrap_or("")
}

fn is_alias(path: &str) -> bool {
    path.starts_with("@/") || path.starts_with("~/")
}

fn is_deep_relative(path: &str) -> bool {
    path.contains("../..")
}

/// `export type { A, B }`: the `type` keyword sits directly on the export
/// statement. In `export type A = …` it belongs to the declaration instead.
fn is_type_only_list(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    let has_clause = node
        .named_children(&mut cursor)
        .any(|child| child.kind() == "export_clause");
    has_clause && has_anonymous(node, "type")
}

fn has_anonymous(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    let found = node
        .children(&mut cursor)
        .any(|child| !child.is_named() && child.kind() == kind);
    found
}
