//! Compiled tree-sitter queries, one pair per grammar.
//!
//! TypeScript and TSX are different grammars, so a query compiled for one
//! cannot run on a tree from the other. JavaScript is parsed with the TSX
//! grammar and therefore shares its compiled query.

use std::sync::OnceLock;

use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use super::SupportedLanguage;

/// A query compiled once for the TypeScript grammar and once for TSX.
pub(crate) struct ScriptQuery {
    source: &'static str,
    typescript: OnceLock<Query>,
    tsx: OnceLock<Query>,
}

impl ScriptQuery {
    pub(crate) const fn new(source: &'static str) -> Self {
        Self {
            source,
            typescript: OnceLock::new(),
            tsx: OnceLock::new(),
        }
    }

    /// The compiled query for `lang`, or `None` when `lang` is not a script language.
    pub(crate) fn get(&self, lang: SupportedLanguage) -> Option<&Query> {
        let slot = match lang {
            SupportedLanguage::TypeScript => &self.typescript,
            SupportedLanguage::Tsx | SupportedLanguage::JavaScript => &self.tsx,
            _ => return None,
        };
        Some(slot.get_or_init(|| compile(lang.grammar(), self.source)))
    }
}

pub(crate) fn compile(lang: SupportedLanguage, source: &str) -> Query {
    Query::new(&lang.language(), source).unwrap_or_else(|err| {
        panic!(
            "tree-sitter query failed to compile for {}: {err:?}\n{source}",
            lang.name()
        )
    })
}

/// Every capture in `tree`, as `(capture_index, node)`.
///
/// Nodes are copied out of the streaming iterator, which would otherwise
/// invalidate them on the next step.
pub(crate) fn captures<'a>(query: &Query, tree: &'a Tree, source: &str) -> Vec<(usize, Node<'a>)> {
    matches(query, tree, source).into_iter().flatten().collect()
}

/// Captures grouped by match, so a name and the node that carried it stay together.
pub(crate) fn matches<'a>(
    query: &Query,
    tree: &'a Tree,
    source: &str,
) -> Vec<Vec<(usize, Node<'a>)>> {
    let mut cursor = QueryCursor::new();
    let mut found = cursor.matches(query, tree.root_node(), source.as_bytes());
    let mut groups = Vec::new();
    while let Some(matched) = found.next() {
        groups.push(
            matched
                .captures()
                .iter()
                .map(|capture| (capture.index as usize, capture.node))
                .collect(),
        );
    }
    groups
}

pub(crate) fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

/// Text of a string node with one layer of quotes removed.
pub(crate) fn unquote(raw: &str) -> &str {
    let trimmed = raw.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2 {
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if (first == b'"' && last == b'"')
            || (first == b'\'' && last == b'\'')
            || (first == b'`' && last == b'`')
        {
            return &trimmed[1..trimmed.len() - 1];
        }
    }
    trimmed
}
