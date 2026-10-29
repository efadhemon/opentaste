//! Function syntax and early-return guards.

use tree_sitter::{Node, Tree};

use super::{run, Count};
use crate::ast::query::ScriptQuery;
use crate::ast::SupportedLanguage;

static SYNTAX: ScriptQuery = ScriptQuery::new(
    r#"
    (function_declaration name: (identifier) @name) @function
    (lexical_declaration
      "const"
      (variable_declarator
        name: (identifier) @name
        value: (arrow_function) @arrow)) @arrow_binding
    "#,
);

static BODIES: ScriptQuery = ScriptQuery::new(
    r#"
    (function_declaration body: (statement_block) @body)
    (method_definition body: (statement_block) @body)
    (lexical_declaration
      "const"
      (variable_declarator
        value: (arrow_function body: (statement_block) @body)))
    "#,
);

pub(super) fn function_syntax(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut declarations = 0u32;
    let mut arrows = 0u32;
    for (index, _) in run(&SYNTAX, lang, tree, source) {
        match capture(&SYNTAX, lang, index) {
            "function" => declarations += 1,
            "arrow_binding" => arrows += 1,
            _ => {}
        }
    }
    Count::from_sides(declarations, arrows)
}

pub(super) fn control_flow(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut guards = 0u32;
    let mut nested = 0u32;
    for (index, node) in run(&BODIES, lang, tree, source) {
        if capture(&BODIES, lang, index) != "body" {
            continue;
        }
        let (has_guard, has_else) = inspect_body(node);
        if has_guard {
            guards += 1;
        }
        if has_else {
            nested += 1;
        }
    }
    Count::from_sides(guards, nested)
}

fn capture(query: &ScriptQuery, lang: SupportedLanguage, index: usize) -> &str {
    query
        .get(lang)
        .and_then(|compiled| compiled.capture_names().get(index).copied())
        .unwrap_or("")
}

fn inspect_body(body: Node<'_>) -> (bool, bool) {
    let mut has_guard = false;
    let mut has_else = false;
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        visit(child, &mut has_guard, &mut has_else);
    }
    (has_guard, has_else)
}

fn visit(node: Node<'_>, has_guard: &mut bool, has_else: &mut bool) {
    if node.kind() == "if_statement" {
        if node.child_by_field_name("alternative").is_some() {
            *has_else = true;
        } else if consequence_returns(node.child_by_field_name("consequence")) {
            *has_guard = true;
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        visit(child, has_guard, has_else);
    }
}

fn consequence_returns(consequence: Option<Node<'_>>) -> bool {
    let Some(node) = consequence else {
        return false;
    };
    match node.kind() {
        "return_statement" | "throw_statement" => true,
        "statement_block" => {
            let mut cursor = node.walk();
            let statements: Vec<_> = node.named_children(&mut cursor).collect();
            !statements.is_empty()
                && statements.iter().all(|statement| {
                    matches!(statement.kind(), "return_statement" | "throw_statement")
                })
        }
        _ => false,
    }
}
