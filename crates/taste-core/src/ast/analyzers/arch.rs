//! Decorators and error classes.

use std::collections::HashSet;
use std::sync::OnceLock;

use tree_sitter::{Node, Tree};

use super::{run, Count};
use crate::ast::query::{self, ScriptQuery};
use crate::ast::SupportedLanguage;

static DECORATORS: ScriptQuery = ScriptQuery::new("(decorator) @decorator");

static ERRORS: ScriptQuery = ScriptQuery::new(
    r#"
    (class_declaration name: (type_identifier) @name) @class
    (throw_statement
      (new_expression constructor: (identifier) @constructor)) @throw
    "#,
);

pub(super) fn decorators(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut custom = 0u32;
    let mut framework = 0u32;
    for (_, node) in run(&DECORATORS, lang, tree, source) {
        let Some(name) = decorator_name(node, source).filter(|name| !name.is_empty()) else {
            continue;
        };
        if is_framework_decorator(&name) {
            framework += 1;
        } else {
            custom += 1;
        }
    }
    Count::from_sides(custom, framework)
}

pub(super) fn errors(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let mut custom = 0u32;
    let mut bare = 0u32;
    for (index, node) in run(&ERRORS, lang, tree, source) {
        match capture(index, lang) {
            "class" => {
                if extends_error_base(node, source) {
                    custom += 1;
                }
            }
            "constructor" => match classify_throw(query::node_text(node, source)) {
                ThrowKind::Custom => custom += 1,
                ThrowKind::Bare => bare += 1,
                ThrowKind::Ignore => {}
            },
            _ => {}
        }
    }
    Count::from_sides(custom, bare)
}

fn capture(index: usize, lang: SupportedLanguage) -> &'static str {
    ERRORS
        .get(lang)
        .and_then(|compiled| compiled.capture_names().get(index).copied())
        .unwrap_or("")
}

enum ThrowKind {
    Custom,
    Bare,
    Ignore,
}

fn classify_throw(name: &str) -> ThrowKind {
    if matches!(name, "Error" | "HttpException") {
        ThrowKind::Bare
    } else if is_builtin_error(name) {
        ThrowKind::Ignore
    } else if name.ends_with("Error") || name.ends_with("Exception") {
        ThrowKind::Custom
    } else {
        ThrowKind::Ignore
    }
}

fn is_builtin_error(name: &str) -> bool {
    matches!(
        name,
        "TypeError" | "RangeError" | "SyntaxError" | "ReferenceError" | "EvalError" | "URIError"
    )
}

fn extends_error_base(class_node: Node<'_>, source: &str) -> bool {
    let mut cursor = class_node.walk();
    for child in class_node.named_children(&mut cursor) {
        if child.kind() != "class_heritage" {
            continue;
        }
        let mut heritage = child.walk();
        for clause in child.named_children(&mut heritage) {
            if clause.kind() != "extends_clause" {
                continue;
            }
            if let Some(base) = clause.child_by_field_name("value") {
                let text = query::node_text(base, source);
                let ident = text.split('.').next_back().unwrap_or(text);
                if matches!(ident, "Error" | "HttpException") {
                    return true;
                }
            }
        }
    }
    false
}

pub(crate) fn decorator_name(node: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let child = node.named_children(&mut cursor).next()?;
    Some(callee_name(child, source))
}

fn callee_name(node: Node<'_>, source: &str) -> String {
    match node.kind() {
        "identifier" | "property_identifier" | "type_identifier" => {
            query::node_text(node, source).to_string()
        }
        "call_expression" => node
            .child_by_field_name("function")
            .map(|function| callee_name(function, source))
            .unwrap_or_default(),
        "member_expression" => node
            .child_by_field_name("property")
            .map(|property| callee_name(property, source))
            .unwrap_or_default(),
        "parenthesized_expression" => {
            let mut cursor = node.walk();
            let inner = node.named_children(&mut cursor).next();
            inner
                .map(|inner| callee_name(inner, source))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn is_framework_decorator(name: &str) -> bool {
    framework_names().contains(name)
}

fn framework_names() -> &'static HashSet<&'static str> {
    static NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        [
            "ApiOperation",
            "ApiProperty",
            "ApiResponse",
            "ApiTags",
            "Body",
            "Catch",
            "Column",
            "Component",
            "Controller",
            "Delete",
            "Directive",
            "Entity",
            "Get",
            "Headers",
            "HostBinding",
            "HostListener",
            "Index",
            "Injectable",
            "Input",
            "IsBoolean",
            "IsEmail",
            "IsNumber",
            "IsOptional",
            "IsString",
            "JoinColumn",
            "JoinTable",
            "ManyToMany",
            "ManyToOne",
            "Max",
            "Min",
            "Module",
            "NgModule",
            "OneToMany",
            "OneToOne",
            "Output",
            "Param",
            "Patch",
            "Pipe",
            "Post",
            "PrimaryColumn",
            "PrimaryGeneratedColumn",
            "Put",
            "Query",
            "Req",
            "Res",
            "UseGuards",
            "UseInterceptors",
            "UsePipes",
            "ValidateNested",
            "ViewChild",
        ]
        .into_iter()
        .collect()
    })
}
