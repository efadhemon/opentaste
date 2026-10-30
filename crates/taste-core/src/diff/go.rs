//! Structural edits in Go files.
//!
//! The three swaps the scan counts, with its narrowing: only an error that
//! came straight from another package's call is "bare", prints in a command
//! (`package main`, `cmd/`, `func main`) are not replaced logging, and
//! `if err != nil` is never the guard that was introduced.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Tree};

use super::structural::{observation, Detector};
use crate::ast::go::{
    all_nodes, flow_of, in_main, is_bare_return, is_function, is_print, is_structured_log,
    is_wrapping, last_returned, File, Flow,
};
use crate::ast::{
    node_text, GO_PREFER_GUARD_CLAUSES, GO_PREFER_STRUCTURED_LOGGING, GO_PREFER_WRAPPED_ERRORS,
};
use crate::models::RuleCategory;
use crate::storage::Observation;

pub(super) const DETECTORS: &[&dyn Detector] = &[&BareToWrapped, &PrintToStructured, &GuardClause];

struct BareToWrapped;
struct PrintToStructured;
struct GuardClause;

/// A `return`, keyed by its function and every value before the error, so
/// `return nil, err` pairs with `return nil, fmt.Errorf(…, err)`.
fn returns<'a>(tree: &'a Tree, source: &str) -> Vec<(String, Node<'a>)> {
    all_nodes(tree)
        .into_iter()
        .filter(|node| node.kind() == "return_statement")
        .filter_map(|ret| {
            let last = last_returned(ret)?;
            let list = ret.named_child(0)?;
            let prefix = &source[list.start_byte()..last.start_byte()];
            let prefix: String = prefix.split_whitespace().collect();
            Some((format!("{}|{prefix}", enclosing(ret, source)), ret))
        })
        .collect()
}

impl Detector for BareToWrapped {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before_file = File::of(path, before_tree, before_src);
        let after_file = File::of(path, after_tree, after_src);
        let after = returns(after_tree, after_src);
        let mut bare_after: HashMap<&str, usize> = HashMap::new();
        for (key, ret) in &after {
            if is_bare_return(&after_file, *ret, after_src) {
                *bare_after.entry(key.as_str()).or_default() += 1;
            }
        }
        let mut bare_before: HashMap<String, usize> = HashMap::new();
        for (key, ret) in returns(before_tree, before_src) {
            if !is_bare_return(&before_file, ret, before_src) {
                continue;
            }
            let count = bare_before.entry(key.clone()).or_default();
            *count += 1;
            if *count <= bare_after.get(key.as_str()).copied().unwrap_or(0) {
                continue;
            }
            let Some(error) = last_returned(ret).map(|value| node_text(value, before_src)) else {
                continue;
            };
            let wrapped = after.iter().find(|(other, candidate)| {
                *other == key
                    && last_returned(*candidate).is_some_and(|value| {
                        is_wrapping(&after_file, value, after_src)
                            && passes(value, error, after_src)
                    })
            });
            let Some((_, wrapped)) = wrapped else {
                continue;
            };
            return Some(observation(
                RuleCategory::ErrorHandling,
                path,
                GO_PREFER_WRAPPED_ERRORS,
                node_text(ret, before_src),
                node_text(*wrapped, after_src),
            ));
        }
        None
    }
}

/// Whether `call` passes the identifier `name` as one of its arguments.
fn passes(call: Node<'_>, name: &str, source: &str) -> bool {
    let Some(args) = call.child_by_field_name("arguments") else {
        return false;
    };
    let mut cursor = args.walk();
    let found = args
        .named_children(&mut cursor)
        .any(|arg| arg.kind() == "identifier" && node_text(arg, source) == name);
    found
}

/// `(call text, first argument)` for every call `wanted` accepts outside `main`.
fn calls(
    file: &File,
    tree: &Tree,
    source: &str,
    wanted: fn(&File, Node<'_>, &str) -> bool,
) -> Vec<(String, String)> {
    all_nodes(tree)
        .into_iter()
        .filter(|node| {
            node.kind() == "call_expression"
                && !in_main(*node, source)
                && wanted(file, *node, source)
        })
        .map(|call| {
            let first = call
                .child_by_field_name("arguments")
                .and_then(|args| args.named_child(0))
                .map(|arg| node_text(arg, source).to_string())
                .unwrap_or_default();
            (node_text(call, source).to_string(), first)
        })
        .collect()
}

impl Detector for PrintToStructured {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before_file = File::of(path, before_tree, before_src);
        let after_file = File::of(path, after_tree, after_src);
        if before_file.is_command {
            return None;
        }
        let prints = calls(&before_file, before_tree, before_src, is_print);
        let after_prints = calls(&after_file, after_tree, after_src, is_print);
        let logs = calls(&after_file, after_tree, after_src, is_structured_log);
        for (print, message) in &prints {
            if message.is_empty() || after_prints.iter().any(|(kept, _)| kept == print) {
                continue;
            }
            // slog takes the message first and key-value pairs after it, so
            // the message is what survives the rewrite.
            let Some((log, _)) = logs.iter().find(|(_, first)| first == message) else {
                continue;
            };
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                GO_PREFER_STRUCTURED_LOGGING,
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
    for node in all_nodes(tree) {
        if !is_function(node) {
            continue;
        }
        let key = function_name(node, source);
        *seen.entry(key.clone()).or_default() += 1;
        found.insert(
            key,
            (flow_of(node, source), node_text(node, source).to_string()),
        );
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
                GO_PREFER_GUARD_CLAUSES,
                old_text.clone(),
                new_text.clone(),
            ));
        }
        None
    }
}

/// `(*Store).Load` for a method, `Load` for a function.
fn function_name(function: Node<'_>, source: &str) -> String {
    let name = function
        .child_by_field_name("name")
        .map(|name| node_text(name, source))
        .unwrap_or("");
    match function
        .child_by_field_name("receiver")
        .and_then(|receiver| receiver.named_child(0))
        .and_then(|param| param.child_by_field_name("type"))
    {
        Some(receiver) => format!("({}).{name}", node_text(receiver, source)),
        None => name.to_string(),
    }
}

fn enclosing(node: Node<'_>, source: &str) -> String {
    let mut current = node.parent();
    while let Some(parent) = current {
        if is_function(parent) {
            return function_name(parent, source);
        }
        current = parent.parent();
    }
    String::new()
}
