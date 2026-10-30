//! Structural edits in Python files.
//!
//! The same four swaps the scan counts, read from a before and an after
//! buffer. Each one fires only for the exact transformation, and only when the
//! classification the scan uses agrees on both sides: an import rewritten
//! between two packages, `Exception` replaced by `ValueError`, or a `print`
//! deleted while an unrelated logger call is added is not a taste.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Node, Tree};

use super::structural::{observation, Detector};
use crate::ast::python::{
    code_nodes, flow_of, is_bare_exception, is_logger_call, is_print_call, raised_class,
    read_imports, Flow, ImportStyle, Module,
};
use crate::ast::{
    node_text, PY_PREFER_ABSOLUTE_IMPORTS, PY_PREFER_CUSTOM_EXCEPTIONS, PY_PREFER_GUARD_CLAUSES,
    PY_PREFER_LOGGING, PY_PREFER_RELATIVE_IMPORTS,
};
use crate::models::RuleCategory;
use crate::storage::Observation;

pub(super) const DETECTORS: &[&dyn Detector] = &[
    &ImportRewrite,
    &ExceptionClass,
    &PrintToLogger,
    &GuardClause,
];

struct ImportRewrite;
struct ExceptionClass;
struct PrintToLogger;
struct GuardClause;

impl Detector for ImportRewrite {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = read_imports(path, before_tree, before_src);
        let after = read_imports(path, after_tree, after_src);
        for old in &before {
            for name in &old.names {
                let Some(new) = after.iter().find(|import| import.names.contains(name)) else {
                    continue;
                };
                let instruction = match (old.style, new.style) {
                    (ImportStyle::Relative, ImportStyle::FirstParty) => PY_PREFER_ABSOLUTE_IMPORTS,
                    (ImportStyle::FirstParty, ImportStyle::Relative) => PY_PREFER_RELATIVE_IMPORTS,
                    _ => continue,
                };
                return Some(observation(
                    RuleCategory::Imports,
                    path,
                    instruction,
                    old.statement.clone(),
                    new.statement.clone(),
                ));
            }
        }
        None
    }
}

/// A `raise` of a class, keyed by where it is and what it is given, so the
/// same message raised in two functions does not pair across them.
struct Raise {
    class: String,
    text: String,
}

fn raises(tree: &Tree, source: &str) -> HashMap<(String, String), Raise> {
    let mut found: HashMap<(String, String), Raise> = HashMap::new();
    let mut seen: HashMap<(String, String), usize> = HashMap::new();
    for node in code_nodes(tree, source) {
        if node.kind() != "raise_statement" {
            continue;
        }
        let Some(class) = raised_class(node, source) else {
            continue;
        };
        let args = node
            .named_child(0)
            .and_then(|value| value.child_by_field_name("arguments"))
            .map(|args| node_text(args, source))
            .unwrap_or("");
        let key = (enclosing(node, source), args.to_string());
        *seen.entry(key.clone()).or_default() += 1;
        found.insert(
            key,
            Raise {
                class: class.to_string(),
                text: node_text(node, source).to_string(),
            },
        );
    }
    // A key that appears twice cannot say which raise became which.
    found.retain(|key, _| seen.get(key) == Some(&1));
    found
}

impl Detector for ExceptionClass {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before_module = Module::of(path, before_tree, before_src);
        let after_module = Module::of(path, after_tree, after_src);
        let before = raises(before_tree, before_src);
        let after = raises(after_tree, after_src);
        let mut keys: Vec<_> = before.keys().collect();
        keys.sort();
        for key in keys {
            let (Some(old), Some(new)) = (before.get(key), after.get(key)) else {
                continue;
            };
            // Only the project's own classes: `Exception` becoming `ValueError`
            // or a framework's `HTTPException` is a different built-in.
            if !is_bare_exception(&old.class)
                || before_module.origin("Exception").is_some()
                || !after_module.is_project_exception(&new.class)
            {
                continue;
            }
            return Some(observation(
                RuleCategory::ErrorHandling,
                path,
                PY_PREFER_CUSTOM_EXCEPTIONS,
                old.text.clone(),
                new.text.clone(),
            ));
        }
        None
    }
}

struct Call {
    text: String,
    args: String,
}

fn calls(tree: &Tree, source: &str, wanted: fn(Node<'_>, &str) -> bool) -> Vec<Call> {
    code_nodes(tree, source)
        .into_iter()
        .filter(|node| node.kind() == "call" && wanted(*node, source))
        .map(|node| Call {
            text: node_text(node, source).to_string(),
            args: node
                .child_by_field_name("arguments")
                .map(|args| node_text(args, source).to_string())
                .unwrap_or_default(),
        })
        .collect()
}

impl Detector for PrintToLogger {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        // `from rich import print` is not the builtin being replaced.
        if Module::of(path, before_tree, before_src)
            .origin("print")
            .is_some()
        {
            return None;
        }
        let prints = calls(before_tree, before_src, is_print_call);
        let after_prints = calls(after_tree, after_src, is_print_call);
        let loggers = calls(after_tree, after_src, is_logger_call);
        for print in &prints {
            if after_prints.iter().any(|kept| kept.text == print.text) {
                continue;
            }
            let Some(logger) = loggers.iter().find(|call| call.args == print.args) else {
                continue;
            };
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                PY_PREFER_LOGGING,
                print.text.clone(),
                logger.text.clone(),
            ));
        }
        None
    }
}

fn flows(tree: &Tree, source: &str) -> HashMap<String, (Flow, String)> {
    let mut found = HashMap::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for node in code_nodes(tree, source) {
        if node.kind() != "function_definition" {
            continue;
        }
        let Some(name) = node.child_by_field_name("name") else {
            continue;
        };
        let key = format!("{}.{}", enclosing(node, source), node_text(name, source));
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
                PY_PREFER_GUARD_CLAUSES,
                old_text.clone(),
                new_text.clone(),
            ));
        }
        None
    }
}

/// The classes and functions around `node`, outermost first: `Users.create`.
fn enclosing(node: Node<'_>, source: &str) -> String {
    let mut names = Vec::new();
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "function_definition" | "class_definition") {
            if let Some(name) = parent.child_by_field_name("name") {
                names.push(node_text(name, source));
            }
        }
        current = parent.parent();
    }
    names.reverse();
    names.join(".")
}
