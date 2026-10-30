//! Structural edits in Rust files.
//!
//! The four swaps the scan counts, held to the same exclusions: nothing in a
//! test item, in `fn main` or in a binary's files is learned from, and an
//! unwrap the idiom calls for (`.lock().unwrap()`) is never the "before".

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Tree};

use super::structural::{observation, Detector};
use crate::ast::rust::{
    code_nodes, flow_of, in_main, is_binary, is_counted_unwrap, is_log_macro, is_print_macro,
    macro_name, unwrapped, use_paths, use_root, Flow, UseRoot,
};
use crate::ast::{
    node_text, RS_PREFER_CRATE_PATHS, RS_PREFER_GUARD_CLAUSES, RS_PREFER_LOG_MACROS,
    RS_PREFER_QUESTION_MARK, RS_PREFER_SUPER_PATHS,
};
use crate::models::RuleCategory;
use crate::storage::Observation;

pub(super) const DETECTORS: &[&dyn Detector] = &[
    &UsePathRewrite,
    &UnwrapToQuestion,
    &PrintToLog,
    &GuardClause,
];

struct UsePathRewrite;
struct UnwrapToQuestion;
struct PrintToLog;
struct GuardClause;

/// `use` declarations rooted at `crate::` or `super::`, keyed by what they
/// import: the part after the last top-level `::`.
fn rooted_uses(tree: &Tree, source: &str) -> HashMap<String, (UseRoot, String)> {
    let mut found = HashMap::new();
    for node in code_nodes(tree, source) {
        if node.kind() != "use_declaration" {
            continue;
        }
        let (Some(root), Some(argument)) =
            (use_root(node, source), node.child_by_field_name("argument"))
        else {
            continue;
        };
        let key = imported(node_text(argument, source));
        // `use super::*` and `use crate::x::*` say nothing about each other.
        if key == "*" {
            continue;
        }
        found.insert(key.to_string(), (root, node_text(node, source).to_string()));
    }
    found
}

/// `User` for `super::models::User`, `{a, b::C}` for `crate::{a, b::C}`.
fn imported(path: &str) -> &str {
    let mut depth = 0usize;
    let mut last = 0usize;
    let bytes = path.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => depth = depth.saturating_sub(1),
            b':' if depth == 0 && bytes.get(index + 1) == Some(&b':') => last = index + 2,
            _ => {}
        }
    }
    &path[last..]
}

impl Detector for UsePathRewrite {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = rooted_uses(before_tree, before_src);
        let after = rooted_uses(after_tree, after_src);
        let mut keys: Vec<_> = before.keys().collect();
        keys.sort();
        for key in keys {
            let (Some((old_root, old_text)), Some((new_root, new_text))) =
                (before.get(key), after.get(key))
            else {
                continue;
            };
            let instruction = match (old_root, new_root) {
                (UseRoot::Super, UseRoot::Crate) => RS_PREFER_CRATE_PATHS,
                (UseRoot::Crate, UseRoot::Super) => RS_PREFER_SUPER_PATHS,
                _ => continue,
            };
            return Some(observation(
                RuleCategory::Imports,
                path,
                instruction,
                old_text.clone(),
                new_text.clone(),
            ));
        }
        None
    }
}

/// Library-code nodes of `tree`: outside tests, `fn main` and binaries.
fn library_nodes<'a>(path: &Path, tree: &'a Tree, source: &str) -> Vec<Node<'a>> {
    if is_binary(path) {
        return Vec::new();
    }
    code_nodes(tree, source)
        .into_iter()
        .filter(|node| !in_main(*node, source))
        .collect()
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect()
}

impl Detector for UnwrapToQuestion {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let after_nodes = library_nodes(path, after_tree, after_src);
        let after_texts: Vec<String> = after_nodes
            .iter()
            .map(|node| squash(node_text(*node, after_src)))
            .collect();
        for call in library_nodes(path, before_tree, before_src) {
            if !is_counted_unwrap(call, before_src) {
                continue;
            }
            let Some(receiver) = unwrapped(call, before_src) else {
                continue;
            };
            let call_text = squash(node_text(call, before_src));
            if after_texts.contains(&call_text) {
                continue;
            }
            let receiver_text = squash(node_text(receiver, before_src));
            let scope = enclosing(call, before_src);
            let Some(question) = after_nodes.iter().find(|node| {
                node.kind() == "try_expression"
                    && node
                        .named_child(0)
                        .is_some_and(|inner| squash(node_text(inner, after_src)) == receiver_text)
                    && enclosing(**node, after_src) == scope
            }) else {
                continue;
            };
            return Some(observation(
                RuleCategory::ErrorHandling,
                path,
                RS_PREFER_QUESTION_MARK,
                node_text(call, before_src),
                node_text(*question, after_src),
            ));
        }
        None
    }
}

/// `(macro text, token tree text)` for every macro `wanted` accepts.
fn macros(
    path: &Path,
    tree: &Tree,
    source: &str,
    wanted: &dyn Fn(&str) -> bool,
) -> Vec<(String, String)> {
    library_nodes(path, tree, source)
        .into_iter()
        .filter(|node| macro_name(*node, source).is_some_and(wanted))
        .map(|node| {
            let mut cursor = node.walk();
            let args = node
                .named_children(&mut cursor)
                .find(|child| child.kind() == "token_tree")
                .map(|tree| node_text(tree, source).to_string())
                .unwrap_or_default();
            (node_text(node, source).to_string(), args)
        })
        .collect()
}

impl Detector for PrintToLog {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let after_imports = use_paths(after_tree, after_src);
        let prints = macros(path, before_tree, before_src, &is_print_macro);
        let after_prints = macros(path, after_tree, after_src, &is_print_macro);
        let logs = macros(path, after_tree, after_src, &|name| {
            is_log_macro(name, &after_imports)
        });
        for (print, args) in &prints {
            if after_prints.iter().any(|(kept, _)| kept == print) {
                continue;
            }
            let Some((log, _)) = logs.iter().find(|(_, log_args)| log_args == args) else {
                continue;
            };
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                RS_PREFER_LOG_MACROS,
                print.clone(),
                log.clone(),
            ));
        }
        None
    }
}

fn flows(tree: &Tree, source: &str) -> HashMap<String, (Flow, String)> {
    let mut found = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for node in code_nodes(tree, source) {
        if node.kind() != "function_item" {
            continue;
        }
        let Some(name) = node.child_by_field_name("name") else {
            continue;
        };
        let key = format!("{}::{}", enclosing(node, source), node_text(name, source));
        *seen.entry(key.clone()).or_default() += 1;
        found.insert(key, (flow_of(node), node_text(node, source).to_string()));
    }
    found.retain(|key, _| seen.get(key) == Some(&1));
    found
}

impl Detector for GuardClause {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = flows(before_tree, before_src);
        let after = flows(after_tree, after_src);
        let mut names: Vec<_> = before.keys().collect();
        names.sort();
        for name in names {
            let (Some((old, old_text)), Some((new, new_text))) =
                (before.get(name), after.get(name))
            else {
                continue;
            };
            if !(old.has_else && new.has_guard && !new.has_else) {
                continue;
            }
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                RS_PREFER_GUARD_CLAUSES,
                old_text.clone(),
                new_text.clone(),
            ));
        }
        None
    }
}

/// The modules, impls and functions around `node`: `store::impl Store::open`.
fn enclosing(node: Node<'_>, source: &str) -> String {
    let mut names = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        let name = match parent.kind() {
            "function_item" | "mod_item" | "trait_item" => parent.child_by_field_name("name"),
            "impl_item" => parent.child_by_field_name("type"),
            _ => None,
        };
        if let Some(name) = name {
            names.push(node_text(name, source));
        }
        current = parent.parent();
    }
    names.reverse();
    names.join("::")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_imported_part_of_a_path_is_what_follows_its_last_top_level_separator() {
        assert_eq!(imported("super::models::User"), "User");
        assert_eq!(
            imported("crate::{db, config::Paths}"),
            "{db, config::Paths}"
        );
        assert_eq!(imported("crate::store::*"), "*");
    }
}
