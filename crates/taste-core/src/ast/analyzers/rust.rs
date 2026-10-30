//! Rust conventions.
//!
//! Only library code counts. Items marked `#[test]` or `#[cfg(test)]` are
//! skipped for every dimension. `.unwrap()` and `println!` are also not
//! counted inside `fn main` or in a binary's files (`main.rs`, `src/bin/`),
//! where panicking on bad input and printing the result are the job.
//!
//! Each count is narrowed to the choice it names. `if … else` that produces a
//! value (`let v = if a { 1 } else { 2 };`, or a function's tail) is Rust's
//! conditional expression, not nested control flow. `.lock().unwrap()` and
//! an `.unwrap()` inside a lazily initialised static are the idiom, not a
//! missing `?`. `info!` is only a log macro when it is qualified with
//! `tracing::` or `log::`, or the file imports it from there.

use std::path::{Component, Path};

use tree_sitter::{Node, Tree};

use super::{Count, RustTally};
use crate::ast::query::node_text;

/// Count every Rust dimension in one file.
pub(super) fn tally(path: &Path, tree: &Tree, source: &str) -> RustTally {
    let nodes = code_nodes(tree, source);
    let binary = is_binary(path);
    let library: Vec<Node<'_>> = if binary {
        Vec::new()
    } else {
        nodes
            .iter()
            .copied()
            .filter(|node| !in_main(*node, source))
            .collect()
    };
    RustTally {
        imports: imports(&nodes, source),
        errors: errors(&library, source),
        logging: logging(tree, &library, source),
        control_flow: control_flow(&nodes, source),
    }
}

/// A binary's own files, where `main` lives.
pub(crate) fn is_binary(path: &Path) -> bool {
    let file = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    let in_bin = path
        .components()
        .any(|part| matches!(part, Component::Normal(name) if name == "bin"));
    file == "main.rs" || in_bin
}

/// Every node outside test items, in document order.
pub(crate) fn code_nodes<'a>(tree: &'a Tree, source: &str) -> Vec<Node<'a>> {
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_test_item(node, source) {
            continue;
        }
        found.push(node);
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    found
}

/// An item preceded by `#[test]`, `#[cfg(test)]` or a runtime's `#[…::test]`.
pub(crate) fn is_test_item(node: Node<'_>, source: &str) -> bool {
    if !matches!(node.kind(), "function_item" | "mod_item" | "impl_item") {
        return false;
    }
    let mut previous = node.prev_named_sibling();
    while let Some(sibling) = previous {
        if sibling.kind() != "attribute_item" {
            break;
        }
        let text: String = node_text(sibling, source).split_whitespace().collect();
        if text == "#[test]" || text.contains("cfg(test)") || text.ends_with("::test]") {
            return true;
        }
        previous = sibling.prev_named_sibling();
    }
    false
}

/// Whether `node` is inside a top-level `fn main`.
pub(crate) fn in_main(node: Node<'_>, source: &str) -> bool {
    let mut current = Some(node);
    while let Some(item) = current {
        if item.kind() == "function_item"
            && item
                .parent()
                .is_some_and(|parent| parent.kind() == "source_file")
            && item
                .child_by_field_name("name")
                .is_some_and(|name| node_text(name, source) == "main")
        {
            return true;
        }
        current = item.parent();
    }
    false
}

/// `crate::` or `super::` at the root of a `use` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UseRoot {
    Crate,
    Super,
}

pub(crate) fn use_root(declaration: Node<'_>, source: &str) -> Option<UseRoot> {
    let argument = declaration.child_by_field_name("argument")?;
    let text = node_text(argument, source);
    if text.starts_with("crate::") {
        Some(UseRoot::Crate)
    } else if text.starts_with("super::") {
        Some(UseRoot::Super)
    } else {
        None
    }
}

fn imports(nodes: &[Node<'_>], source: &str) -> Count {
    let mut crate_paths = 0u32;
    let mut super_paths = 0u32;
    for node in nodes.iter().filter(|node| node.kind() == "use_declaration") {
        match use_root(*node, source) {
            Some(UseRoot::Crate) => crate_paths += 1,
            Some(UseRoot::Super) => super_paths += 1,
            None => {}
        }
    }
    Count::from_sides(crate_paths, super_paths)
}

/// `receiver.unwrap()` or `receiver.expect(…)`, returning the receiver.
pub(crate) fn unwrapped<'a>(call: Node<'a>, source: &str) -> Option<Node<'a>> {
    if call.kind() != "call_expression" {
        return None;
    }
    let function = call.child_by_field_name("function")?;
    if function.kind() != "field_expression" {
        return None;
    }
    let field = function.child_by_field_name("field")?;
    if !matches!(node_text(field, source), "unwrap" | "expect") {
        return None;
    }
    function.child_by_field_name("value")
}

/// An unwrap the idiom calls for: on a lock, or while building a lazily
/// initialised value, where there is no caller to hand an error to.
fn is_idiomatic_unwrap(call: Node<'_>, receiver: Node<'_>, source: &str) -> bool {
    if receiver.kind() == "call_expression" {
        let method = receiver
            .child_by_field_name("function")
            .filter(|function| function.kind() == "field_expression")
            .and_then(|function| function.child_by_field_name("field"))
            .map(|field| node_text(field, source));
        if matches!(method, Some("lock" | "read" | "write")) {
            return true;
        }
    }
    let mut current = call.parent();
    while let Some(node) = current {
        match node.kind() {
            "static_item" | "const_item" => return true,
            "closure_expression" => {
                let called = node
                    .parent()
                    .filter(|parent| parent.kind() == "arguments")
                    .and_then(|args| args.parent())
                    .and_then(|outer| outer.child_by_field_name("function"))
                    .map(|function| node_text(function, source));
                if called.is_some_and(|called| {
                    called.ends_with("get_or_init") || called.contains("Lazy")
                }) {
                    return true;
                }
            }
            "function_item" => return false,
            _ => {}
        }
        current = node.parent();
    }
    false
}

/// Whether `call` is an unwrap that counts against `?`.
pub(crate) fn is_counted_unwrap(call: Node<'_>, source: &str) -> bool {
    unwrapped(call, source).is_some_and(|receiver| !is_idiomatic_unwrap(call, receiver, source))
}

fn errors(library: &[Node<'_>], source: &str) -> Count {
    let mut propagated = 0u32;
    let mut unwraps = 0u32;
    for node in library {
        if node.kind() == "try_expression" {
            propagated += 1;
        } else if is_counted_unwrap(*node, source) {
            unwraps += 1;
        }
    }
    Count::from_sides(propagated, unwraps)
}

const PRINT_MACROS: &[&str] = &["println", "eprintln", "print", "eprint"];
const LOG_MACROS: &[&str] = &["trace", "debug", "info", "warn", "error", "event"];
const LOG_CRATES: &[&str] = &["tracing", "log"];

/// The macro a `macro_invocation` calls, as written: `info`, `tracing::info`.
pub(crate) fn macro_name<'a>(invocation: Node<'_>, source: &'a str) -> Option<&'a str> {
    (invocation.kind() == "macro_invocation")
        .then(|| invocation.child_by_field_name("macro"))
        .flatten()
        .map(|name| node_text(name, source))
}

pub(crate) fn is_print_macro(name: &str) -> bool {
    PRINT_MACROS.contains(&name) || name.strip_prefix("std::").is_some_and(is_print_macro)
}

/// Whether `name` is a `tracing` or `log` macro in a file with `imports`.
pub(crate) fn is_log_macro(name: &str, imports: &[String]) -> bool {
    if let Some((krate, macro_name)) = name.split_once("::") {
        return LOG_CRATES.contains(&krate) && LOG_MACROS.contains(&macro_name);
    }
    LOG_MACROS.contains(&name)
        && imports.iter().any(|import| {
            LOG_CRATES
                .iter()
                .any(|krate| import.starts_with(&format!("{krate}::")))
                && (import.contains('*') || mentions(import, name))
        })
}

fn mentions(import: &str, name: &str) -> bool {
    import
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == name)
}

/// The argument text of every `use` in the file.
pub(crate) fn use_paths(tree: &Tree, source: &str) -> Vec<String> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    let mut paths = Vec::new();
    let mut stack: Vec<Node<'_>> = root.named_children(&mut cursor).collect();
    while let Some(node) = stack.pop() {
        if node.kind() == "use_declaration" {
            if let Some(argument) = node.child_by_field_name("argument") {
                paths.push(node_text(argument, source).to_string());
            }
        } else if node.kind() == "mod_item" {
            if let Some(body) = node.child_by_field_name("body") {
                let mut inner = body.walk();
                stack.extend(body.named_children(&mut inner));
            }
        }
    }
    paths
}

fn logging(tree: &Tree, library: &[Node<'_>], source: &str) -> Count {
    let imports = use_paths(tree, source);
    let mut logged = 0u32;
    let mut printed = 0u32;
    for node in library {
        let Some(name) = macro_name(*node, source) else {
            continue;
        };
        if is_log_macro(name, &imports) {
            logged += 1;
        } else if is_print_macro(name) {
            printed += 1;
        }
    }
    Count::from_sides(logged, printed)
}

/// Whether a function body has an early-exit guard and whether it has an
/// `if … else` used as control flow. Nested functions are their own bodies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flow {
    pub(crate) has_guard: bool,
    pub(crate) has_else: bool,
}

pub(crate) fn flow_of(function: Node<'_>) -> Flow {
    let mut flow = Flow::default();
    if let Some(body) = function.child_by_field_name("body") {
        visit_flow(body, &mut flow);
    }
    flow
}

fn visit_flow(node: Node<'_>, flow: &mut Flow) {
    match node.kind() {
        // `let … else` must diverge: it is a guard by construction.
        "let_declaration" if node.child_by_field_name("alternative").is_some() => {
            flow.has_guard = true;
        }
        "if_expression"
            if node
                .parent()
                .is_some_and(|parent| parent.kind() == "expression_statement") =>
        {
            match node.child_by_field_name("alternative") {
                Some(_) if !yields_value(node) => flow.has_else = true,
                Some(_) => {}
                None if returns(node.child_by_field_name("consequence")) => {
                    flow.has_guard = true;
                }
                None => {}
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "function_item" | "closure_expression") {
            continue;
        }
        visit_flow(child, flow);
    }
}

/// Whether any branch of an `if … else` chain ends in a value rather than a
/// statement, which makes it a conditional expression.
fn yields_value(if_expression: Node<'_>) -> bool {
    let mut branches = Vec::new();
    let mut current = Some(if_expression);
    while let Some(node) = current {
        branches.extend(node.child_by_field_name("consequence"));
        current = None;
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let mut cursor = alternative.walk();
            for child in alternative.named_children(&mut cursor) {
                match child.kind() {
                    "if_expression" => current = Some(child),
                    "block" => branches.push(child),
                    _ => {}
                }
            }
        }
    }
    branches.iter().any(|block| {
        block
            .named_child(block.named_child_count().saturating_sub(1) as u32)
            .is_some_and(|last| {
                // `return`, `break` and `continue` diverge; they are not values.
                !matches!(
                    last.kind(),
                    "expression_statement"
                        | "let_declaration"
                        | "line_comment"
                        | "block_comment"
                        | "return_expression"
                        | "break_expression"
                        | "continue_expression"
                ) && !last.kind().ends_with("_item")
            })
    })
}

/// A block whose only statement is `return …`.
fn returns(block: Option<Node<'_>>) -> bool {
    let Some(block) = block else {
        return false;
    };
    let mut cursor = block.walk();
    let statements: Vec<_> = block
        .named_children(&mut cursor)
        .filter(|child| !child.kind().ends_with("comment"))
        .collect();
    let [only] = statements.as_slice() else {
        return false;
    };
    let expression = if only.kind() == "expression_statement" {
        only.named_child(0)
    } else {
        Some(*only)
    };
    expression.is_some_and(|expression| expression.kind() == "return_expression")
}

fn control_flow(nodes: &[Node<'_>], _source: &str) -> Count {
    let mut guards = 0u32;
    let mut nested = 0u32;
    for function in nodes.iter().filter(|node| node.kind() == "function_item") {
        let flow = flow_of(*function);
        guards += u32::from(flow.has_guard);
        nested += u32::from(flow.has_else);
    }
    Count::from_sides(guards, nested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, SupportedLanguage};

    fn tally_at(path: &str, source: &str) -> RustTally {
        let tree = parse(SupportedLanguage::Rust, source).unwrap();
        tally(Path::new(path), &tree, source)
    }

    fn tally_of(source: &str) -> RustTally {
        tally_at("src/store.rs", source)
    }

    #[test]
    fn crate_paths_count_against_super_paths() {
        let unanimous = tally_of("use crate::models::User;\nuse crate::{db, config::Paths};\n");
        assert_eq!(unanimous.imports, Count::from_sides(2, 0));

        let mixed = tally_of("use crate::models::User;\nuse super::db;\nuse super::super::x;\n");
        assert_eq!(mixed.imports, Count::from_sides(1, 2));

        let nothing = tally_of(
            "use std::io;\nuse serde::Serialize;\nuse self::inner::X;\n\
             #[cfg(test)]\nmod tests {\n    use super::*;\n}\n",
        );
        assert_eq!(
            nothing.imports,
            Count::default(),
            "test modules' `use super::*` is the idiom"
        );
    }

    #[test]
    fn question_marks_count_against_unwrap_and_expect() {
        let unanimous = tally_of(
            "fn f() -> Result<(), E> {\n    let a = load()?;\n    a.save()?;\n    Ok(())\n}\n",
        );
        assert_eq!(unanimous.errors, Count::from_sides(2, 0));

        let mixed = tally_of(
            "fn f() -> Result<(), E> {\n    let a = load()?;\n    let b = a.get().unwrap();\n    b.set().expect(\"set\");\n    Ok(())\n}\n",
        );
        assert_eq!(mixed.errors, Count::from_sides(1, 2));

        let nothing = tally_of(
            "static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(\"x\").unwrap());\n\
             fn re() -> &'static Regex {\n    static R: OnceLock<Regex> = OnceLock::new();\n    R.get_or_init(|| Regex::new(\"y\").unwrap())\n}\n\
             fn g(m: &Mutex<u8>) {\n    let a = m.lock().unwrap();\n    let b = rw.read().expect(\"poisoned\");\n    let c = x.unwrap_or(1);\n}\n\
             fn main() {\n    run().unwrap();\n}\n\
             #[test]\nfn t() {\n    f().unwrap();\n}\n\
             #[cfg(test)]\nmod tests {\n    fn h() { g().unwrap(); }\n}\n",
        );
        assert_eq!(nothing.errors, Count::default());
    }

    #[test]
    fn a_binarys_files_do_not_count_unwrap_or_println() {
        let source = "fn run() {\n    load().unwrap();\n    println!(\"done\");\n}\n";
        for path in [
            "src/main.rs",
            "src/bin/tool.rs",
            "crates/cli/src/bin/x/helpers.rs",
        ] {
            let got = tally_at(path, source);
            assert_eq!(got.errors, Count::default(), "{path}");
            assert_eq!(got.logging, Count::default(), "{path}");
        }
    }

    #[test]
    fn log_macros_count_against_println() {
        let unanimous = tally_of(
            "use tracing::{info, warn};\nfn f() {\n    info!(\"a\");\n    warn!(x, \"b\");\n    log::error!(\"c\");\n}\n",
        );
        assert_eq!(unanimous.logging, Count::from_sides(3, 0));

        let mixed = tally_of(
            "fn f() {\n    tracing::debug!(\"a\");\n    println!(\"b\");\n    eprintln!(\"c\");\n}\n",
        );
        assert_eq!(mixed.logging, Count::from_sides(1, 2));

        let nothing = tally_of(
            "macro_rules! info { () => {} }\nfn f() {\n    info!();\n    dbg!(x);\n    write!(out, \"x\");\n}\n\
             fn main() {\n    println!(\"usage\");\n}\n",
        );
        assert_eq!(
            nothing.logging,
            Count::default(),
            "an unimported `info!` may be the file's own macro"
        );
    }

    #[test]
    fn guards_and_let_else_count_against_if_else_statements() {
        let unanimous = tally_of(
            "fn f(x: Option<u8>) -> u8 {\n    let Some(y) = x else { return 0; };\n    y\n}\n\
             fn g(x: u8) -> u8 {\n    if x == 0 {\n        return 1;\n    }\n    x\n}\n",
        );
        assert_eq!(unanimous.control_flow, Count::from_sides(2, 0));

        let mixed = tally_of(
            "fn f(x: Option<u8>) -> u8 {\n    let Some(y) = x else { return 0; };\n    y\n}\n\
             fn g(x: u8) {\n    if x == 0 {\n        a();\n    } else {\n        b();\n    }\n}\n\
             fn h(x: u8) -> u8 {\n    if x == 0 {\n        return 1\n    } else {\n        return 2\n    }\n}\n",
        );
        assert_eq!(mixed.control_flow, Count::from_sides(1, 2));

        let nothing = tally_of(
            "fn f(x: u8) -> u8 {\n    let v = if x > 1 { 2 } else { 3 };\n    v\n}\n\
             fn g(x: u8) -> u8 {\n    if x > 1 {\n        2\n    } else if x > 0 {\n        1\n    } else {\n        0\n    }\n}\n\
             fn h(x: u8) {\n    if x > 1 {\n        a();\n    }\n}\n",
        );
        assert_eq!(
            nothing.control_flow,
            Count::default(),
            "if/else producing a value is an expression, and a plain if is no guard"
        );
    }
}
