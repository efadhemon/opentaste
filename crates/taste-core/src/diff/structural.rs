//! Concrete structural edits between a before and an after buffer.
//!
//! Every snippet this module produces passes through [`super::scrub`] inside
//! [`analyze`]. Detectors return the raw node text; they do not redact it
//! themselves, so a new detector cannot skip the scrubber.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::{decorator_name, node_text, parse, unquote, ScriptQuery, SupportedLanguage};
use crate::models::{RuleCategory, RuleSource};
use crate::storage::{validate_scope, Observation};

use super::scrub;

const ROLES: &[&str] = &[
    "controller",
    "dto",
    "e2e",
    "entity",
    "gateway",
    "guard",
    "middleware",
    "module",
    "pipe",
    "repository",
    "resolver",
    "service",
    "spec",
    "test",
];

/// Compare `before` with `after` and return the edits that are unambiguous.
///
/// `path` is the project-relative file the edit belongs to. The scope glob is
/// derived from it (`src/users/user.entity.ts` becomes `src/**/*.entity.ts`).
pub fn analyze(before: &str, after: &str, path: &Path) -> crate::error::Result<Vec<Observation>> {
    let Some(lang) = SupportedLanguage::from_path(path) else {
        return Ok(Vec::new());
    };
    if !lang.is_script() {
        return Ok(Vec::new());
    }
    let (Some(before_tree), Some(after_tree)) = (parse(lang, before), parse(lang, after)) else {
        return Ok(Vec::new());
    };

    let (removed, added) = changed_lines(before, after);
    let mut found = Vec::new();
    for detector in DETECTORS {
        let Some(observation) = detector.detect(&before_tree, before, &after_tree, after, path)
        else {
            continue;
        };
        if !grounded(&observation, before, after, &removed, &added) {
            continue;
        }
        found.push(scrub_observation(observation, lang));
    }
    Ok(found)
}

trait Detector: Sync {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation>;
}

const DETECTORS: &[&dyn Detector] = &[
    &DecoratorSwap,
    &ImportRewrite,
    &LoggerReplacement,
    &TypeSwap,
    &FunctionSyntax,
    &ErrorClass,
    &ControlFlow,
];

struct DecoratorSwap;
struct ImportRewrite;
struct LoggerReplacement;
struct TypeSwap;
struct FunctionSyntax;
struct ErrorClass;
struct ControlFlow;

impl Detector for DecoratorSwap {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = decorated(before_tree, before_src);
        let after = decorated(after_tree, after_src);
        let mut keys: Vec<_> = before.keys().chain(after.keys()).cloned().collect();
        keys.sort();
        keys.dedup();
        for key in keys {
            let (Some(before_item), Some(after_item)) = (before.get(&key), after.get(&key)) else {
                continue;
            };
            let Some(swap) = swapped_names(before_item, after_item) else {
                continue;
            };
            let (old_name, old_text, new_name, new_text) =
                (swap.old_name, swap.old_text, swap.new_name, swap.new_text);
            return Some(observation(
                RuleCategory::Architecture,
                path,
                format!("Use `@{new_name}` instead of `@{old_name}`."),
                old_text,
                new_text,
            ));
        }
        None
    }
}

impl Detector for ImportRewrite {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = imports(before_tree, before_src);
        let after = imports(after_tree, after_src);
        for (name, before_import) in &before {
            let Some(after_import) = after.get(name) else {
                continue;
            };
            let (Some(from), Some(to)) = (
                import_class(&before_import.path),
                import_class(&after_import.path),
            ) else {
                continue;
            };
            if from == to {
                continue;
            }
            let instruction = if to == ImportClass::Alias {
                crate::ast::PREFER_PATH_ALIASES
            } else {
                "Prefer relative imports over path aliases (`@/` or `~/`)."
            };
            return Some(observation(
                RuleCategory::Imports,
                path,
                instruction,
                before_import.statement.clone(),
                after_import.statement.clone(),
            ));
        }
        None
    }
}

impl Detector for LoggerReplacement {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = calls(before_tree, before_src);
        let after = calls(after_tree, after_src);
        for console in before.iter().filter(|call| call.kind == CallKind::Console) {
            if after.iter().any(|call| call.text == console.text) {
                continue;
            }
            let logger = after
                .iter()
                .find(|call| call.kind == CallKind::Logger && call.args == console.args)?;
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                "Prefer a logger service over `console.log` and `console.error`.",
                console.text.clone(),
                logger.text.clone(),
            ));
        }
        None
    }
}

impl Detector for TypeSwap {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = type_decls(before_tree, before_src);
        let after = type_decls(after_tree, after_src);
        for (name, before_decl) in &before {
            let Some(after_decl) = after.get(name) else {
                continue;
            };
            if before_decl.kind == after_decl.kind {
                continue;
            }
            let instruction = if after_decl.kind == TypeKind::Alias {
                crate::ast::PREFER_TYPE_ALIAS
            } else {
                crate::ast::PREFER_INTERFACE
            };
            return Some(observation(
                RuleCategory::Types,
                path,
                instruction,
                before_decl.text.clone(),
                after_decl.text.clone(),
            ));
        }
        None
    }
}

impl Detector for FunctionSyntax {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = functions(before_tree, before_src);
        let after = functions(after_tree, after_src);
        for (name, before_fn) in &before {
            let Some(after_fn) = after.get(name) else {
                continue;
            };
            if before_fn.kind == after_fn.kind {
                continue;
            }
            let instruction = if after_fn.kind == FnKind::Arrow {
                crate::ast::PREFER_ARROW_FUNCTIONS
            } else {
                crate::ast::PREFER_FUNCTION_DECLARATIONS
            };
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                instruction,
                before_fn.text.clone(),
                after_fn.text.clone(),
            ));
        }
        None
    }
}

impl Detector for ErrorClass {
    fn detect(
        &self,
        before_tree: &Tree,
        before_src: &str,
        after_tree: &Tree,
        after_src: &str,
        path: &Path,
    ) -> Option<Observation> {
        let before = throws(before_tree, before_src);
        let after = throws(after_tree, after_src);
        for (args, before_throw) in &before {
            let Some(after_throw) = after.get(args) else {
                continue;
            };
            if before_throw.constructor == after_throw.constructor {
                continue;
            }
            if !is_bare_error(&before_throw.constructor) || is_bare_error(&after_throw.constructor)
            {
                continue;
            }
            return Some(observation(
                RuleCategory::ErrorHandling,
                path,
                crate::ast::PREFER_CUSTOM_ERRORS,
                before_throw.text.clone(),
                after_throw.text.clone(),
            ));
        }
        None
    }
}

impl Detector for ControlFlow {
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
        for (name, before_flow) in &before {
            let Some(after_flow) = after.get(name) else {
                continue;
            };
            if !(before_flow.has_else && after_flow.has_guard && !after_flow.has_else) {
                continue;
            }
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                crate::ast::PREFER_GUARD_CLAUSES,
                before_flow.sample.clone(),
                after_flow.sample.clone(),
            ));
        }
        None
    }
}

fn observation(
    category: RuleCategory,
    path: &Path,
    instruction: impl Into<String>,
    anti: impl Into<String>,
    preferred: impl Into<String>,
) -> Observation {
    Observation::new(
        category,
        scope_for_path(path),
        RuleSource::AstDiff,
        instruction,
    )
    .with_patterns(Some(anti.into()), Some(preferred.into()))
}

fn scrub_observation(mut observation: Observation, lang: SupportedLanguage) -> Observation {
    if let Some(anti) = observation.anti_pattern.as_mut() {
        *anti = scrub(anti, lang);
    }
    if let Some(preferred) = observation.preferred_pattern.as_mut() {
        *preferred = scrub(preferred, lang);
    }
    observation
}

fn grounded(
    observation: &Observation,
    before: &str,
    after: &str,
    removed: &HashSet<usize>,
    added: &HashSet<usize>,
) -> bool {
    let Some(anti) = observation.anti_pattern.as_deref() else {
        return false;
    };
    let Some(preferred) = observation.preferred_pattern.as_deref() else {
        return false;
    };
    snippet_on_lines(before, anti, removed) && snippet_on_lines(after, preferred, added)
}

fn changed_lines(before: &str, after: &str) -> (HashSet<usize>, HashSet<usize>) {
    let diff = similar::TextDiff::from_lines(before, after);
    let mut removed = HashSet::new();
    let mut added = HashSet::new();
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Delete => {
                if let Some(index) = change.old_index() {
                    removed.insert(index);
                }
            }
            similar::ChangeTag::Insert => {
                if let Some(index) = change.new_index() {
                    added.insert(index);
                }
            }
            similar::ChangeTag::Equal => {}
        }
    }
    (removed, added)
}

fn snippet_on_lines(source: &str, snippet: &str, lines: &HashSet<usize>) -> bool {
    if snippet.is_empty() {
        return false;
    }
    let mut from = 0;
    while let Some(offset) = source[from..].find(snippet) {
        let start = from + offset;
        let start_line = source[..start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count();
        let end_line = start_line + snippet.bytes().filter(|byte| *byte == b'\n').count();
        if (start_line..=end_line).any(|line| lines.contains(&line)) {
            return true;
        }
        from = start + snippet.len();
    }
    false
}

/// Glob covering files of the same role as `path`.
///
/// `src/users/user.entity.ts` becomes `src/**/*.entity.ts`. A rule scoped to
/// `**/*` would apply the edit to every file in the project, which is a wrong
/// rule even when the edit itself was right.
pub(crate) fn scope_for_path(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    let relative = if normalized.starts_with('/') {
        normalized.rsplit('/').next().unwrap_or(normalized)
    } else {
        normalized
    };
    let (dir, file) = match relative.rfind('/') {
        Some(split) => (&relative[..split], &relative[split + 1..]),
        None => ("", relative),
    };
    let pattern = file_pattern(file);
    let top = dir
        .split('/')
        .find(|part| !part.is_empty() && !glob_meta(part));
    let scope = match top {
        Some(top) => format!("{top}/**/{pattern}"),
        None => format!("**/{pattern}"),
    };
    if validate_scope(&scope).is_ok() {
        scope
    } else {
        format!("**/{pattern}")
    }
}

fn file_pattern(file: &str) -> String {
    let mut parts = file.rsplitn(3, '.');
    let ext = parts.next().unwrap_or("txt");
    let role = parts.next();
    let stem = parts.next();
    if stem.is_some() {
        if let Some(role) = role {
            if ROLES.contains(&role) && !glob_meta(role) && !glob_meta(ext) {
                return format!("*.{role}.{ext}");
            }
        }
    }
    if glob_meta(ext) {
        "*".to_string()
    } else {
        format!("*.{ext}")
    }
}

fn glob_meta(value: &str) -> bool {
    value.contains(['*', '?', '[', ']', '{', '}'])
}

struct Decorated {
    decorators: Vec<(String, String)>,
}

fn decorated(tree: &Tree, source: &str) -> HashMap<String, Decorated> {
    let mut items = HashMap::new();
    walk_decorated(tree.root_node(), source, &mut items);
    items
}

fn walk_decorated(node: Node<'_>, source: &str, items: &mut HashMap<String, Decorated>) {
    if matches!(
        node.kind(),
        "class_declaration" | "abstract_class_declaration" | "class"
    ) {
        if let Some(name_node) = node.child_by_field_name("name") {
            let class_name = node_text(name_node, source).to_string();
            let mut decorators = Vec::new();
            if let Some(parent) = node.parent() {
                if parent.kind() == "export_statement" {
                    decorators.extend(decorators_of(parent, source));
                }
            }
            decorators.extend(decorators_of(node, source));
            if !decorators.is_empty() {
                items.insert(format!("class:{class_name}"), Decorated { decorators });
            }
            if let Some(body) = node.child_by_field_name("body") {
                collect_members(body, &class_name, source, items);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_decorated(child, source, items);
    }
}

fn collect_members(
    body: Node<'_>,
    class_name: &str,
    source: &str,
    items: &mut HashMap<String, Decorated>,
) {
    let mut pending = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "decorator" => {
                if let Some(pair) = named_decorator(child, source) {
                    pending.push(pair);
                }
            }
            "comment" => {}
            "method_definition"
            | "public_field_definition"
            | "method_signature"
            | "abstract_method_signature" => {
                let mut decorators = std::mem::take(&mut pending);
                decorators.extend(decorators_of(child, source));
                if decorators.is_empty() {
                    continue;
                }
                if let Some(name_node) = child.child_by_field_name("name") {
                    let kind = if child.kind() == "public_field_definition" {
                        "field"
                    } else {
                        "method"
                    };
                    items.insert(
                        format!("{kind}:{class_name}.{}", node_text(name_node, source)),
                        Decorated { decorators },
                    );
                }
            }
            _ => pending.clear(),
        }
    }
}

fn decorators_of(node: Node<'_>, source: &str) -> Vec<(String, String)> {
    let mut cursor = node.walk();
    let found = node
        .children_by_field_name("decorator", &mut cursor)
        .filter_map(|decorator| named_decorator(decorator, source))
        .collect();
    found
}

fn named_decorator(node: Node<'_>, source: &str) -> Option<(String, String)> {
    let name = decorator_name(node, source).filter(|name| !name.is_empty())?;
    Some((name, node_text(node, source).to_string()))
}

struct Swap {
    old_name: String,
    old_text: String,
    new_name: String,
    new_text: String,
}

fn swapped_names(before: &Decorated, after: &Decorated) -> Option<Swap> {
    let removed: Vec<_> = before
        .decorators
        .iter()
        .filter(|(name, _)| after.decorators.iter().all(|(other, _)| other != name))
        .collect();
    let added: Vec<_> = after
        .decorators
        .iter()
        .filter(|(name, _)| before.decorators.iter().all(|(other, _)| other != name))
        .collect();
    if removed.len() == 1 && added.len() == 1 {
        let (old_name, old_text) = removed[0].clone();
        let (new_name, new_text) = added[0].clone();
        Some(Swap {
            old_name,
            old_text,
            new_name,
            new_text,
        })
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportClass {
    Alias,
    Relative,
}

struct ImportBinding {
    path: String,
    statement: String,
}

static IMPORTS: ScriptQuery = ScriptQuery::new("(import_statement) @stmt");

fn imports(tree: &Tree, source: &str) -> HashMap<String, ImportBinding> {
    let mut found = HashMap::new();
    for_each_match(&IMPORTS, tree, source, |names, nodes| {
        let stmt = node_of("stmt", names, nodes)?;
        let source_node = stmt.child_by_field_name("source")?;
        let path = unquote(node_text(source_node, source)).to_string();
        let statement = node_text(stmt, source).trim().to_string();
        for name in import_locals(stmt, source) {
            found.entry(name).or_insert_with(|| ImportBinding {
                path: path.clone(),
                statement: statement.clone(),
            });
        }
        Some(())
    });
    found
}

fn import_locals(stmt: Node<'_>, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = stmt.walk();
    for child in stmt.named_children(&mut cursor) {
        if child.kind() == "import_clause" {
            collect_locals(child, source, &mut names);
        }
    }
    names
}

fn collect_locals(node: Node<'_>, source: &str, names: &mut Vec<String>) {
    match node.kind() {
        "import_specifier" => {
            let local = node
                .child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name"));
            if let Some(local) = local {
                names.push(node_text(local, source).to_string());
            }
        }
        "namespace_import" | "identifier" => {
            if node.kind() == "identifier" {
                names.push(node_text(node, source).to_string());
            } else {
                let mut cursor = node.walk();
                let ident = node
                    .named_children(&mut cursor)
                    .find(|child| child.kind() == "identifier");
                if let Some(ident) = ident {
                    names.push(node_text(ident, source).to_string());
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_locals(child, source, names);
            }
        }
    }
}

fn import_class(path: &str) -> Option<ImportClass> {
    if path.starts_with("@/") || path.starts_with("~/") {
        Some(ImportClass::Alias)
    } else if path.starts_with('.') {
        Some(ImportClass::Relative)
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Console,
    Logger,
}

struct Call {
    kind: CallKind,
    args: String,
    text: String,
}

static CALLS: ScriptQuery = ScriptQuery::new(
    r#"
    (call_expression
      function: (member_expression
        object: (_) @object
        property: (property_identifier) @prop)
      arguments: (arguments) @args) @call
    "#,
);

fn calls(tree: &Tree, source: &str) -> Vec<Call> {
    let mut found = Vec::new();
    for_each_match(&CALLS, tree, source, |names, nodes| {
        let object = text_of("object", names, nodes, source)?;
        let prop = text_of("prop", names, nodes, source)?;
        let args = text_of("args", names, nodes, source)?;
        let text = text_of("call", names, nodes, source)?;
        let kind = if object == "console" && is_log_method(prop) {
            CallKind::Console
        } else if (object == "logger" || object.ends_with(".logger")) && is_log_method(prop) {
            CallKind::Logger
        } else {
            return None;
        };
        found.push(Call {
            kind,
            args: args.to_string(),
            text: text.trim().to_string(),
        });
        Some(())
    });
    found
}

fn is_log_method(name: &str) -> bool {
    matches!(name, "log" | "error" | "warn" | "info" | "debug")
}

fn is_bare_error(name: &str) -> bool {
    matches!(name, "Error" | "HttpException")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeKind {
    Interface,
    Alias,
}

struct TypeDecl {
    kind: TypeKind,
    text: String,
}

static TYPES: ScriptQuery = ScriptQuery::new(
    r#"
    (interface_declaration name: (type_identifier) @name) @decl
    (type_alias_declaration name: (type_identifier) @name) @decl
    "#,
);

fn type_decls(tree: &Tree, source: &str) -> HashMap<String, TypeDecl> {
    let mut found = HashMap::new();
    for_each_match(&TYPES, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?.to_string();
        let decl = node_of("decl", names, nodes)?;
        let text = node_text(decl, source).trim().to_string();
        let kind = if decl.kind() == "interface_declaration" {
            TypeKind::Interface
        } else {
            TypeKind::Alias
        };
        found.insert(name, TypeDecl { kind, text });
        Some(())
    });
    found
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FnKind {
    Declaration,
    Arrow,
}

struct FnDecl {
    kind: FnKind,
    text: String,
}

static FUNCTIONS: ScriptQuery = ScriptQuery::new(
    r#"
    (function_declaration name: (identifier) @name) @decl
    (lexical_declaration
      "const"
      (variable_declarator
        name: (identifier) @name
        value: (arrow_function))) @decl
    "#,
);

fn functions(tree: &Tree, source: &str) -> HashMap<String, FnDecl> {
    let mut found = HashMap::new();
    for_each_match(&FUNCTIONS, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?.to_string();
        let decl = node_of("decl", names, nodes)?;
        let kind = if decl.kind() == "function_declaration" {
            FnKind::Declaration
        } else {
            FnKind::Arrow
        };
        found.insert(
            name,
            FnDecl {
                kind,
                text: node_text(decl, source).trim().to_string(),
            },
        );
        Some(())
    });
    found
}

struct Throw {
    constructor: String,
    text: String,
}

static THROWS: ScriptQuery = ScriptQuery::new(
    r#"
    (throw_statement
      (new_expression
        constructor: (identifier) @constructor
        arguments: (arguments) @args)) @throw
    "#,
);

fn throws(tree: &Tree, source: &str) -> HashMap<String, Throw> {
    let mut found = HashMap::new();
    for_each_match(&THROWS, tree, source, |names, nodes| {
        let constructor = text_of("constructor", names, nodes, source)?.to_string();
        let args = text_of("args", names, nodes, source)?.to_string();
        let text = text_of("throw", names, nodes, source)?.trim().to_string();
        found.insert(args, Throw { constructor, text });
        Some(())
    });
    found
}

struct Flow {
    has_else: bool,
    has_guard: bool,
    sample: String,
}

static FLOWS: ScriptQuery = ScriptQuery::new(
    "(function_declaration name: (identifier) @name body: (statement_block) @body) @fn",
);

fn flows(tree: &Tree, source: &str) -> HashMap<String, Flow> {
    let mut found = HashMap::new();
    for_each_match(&FLOWS, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?.to_string();
        let body = node_of("body", names, nodes)?;
        let flow = inspect_flow(body, source);
        found.insert(name, flow);
        Some(())
    });
    found
}

fn inspect_flow(body: Node<'_>, source: &str) -> Flow {
    let mut has_else = false;
    let mut has_guard = false;
    let mut else_text = None;
    let mut guard_text = None;
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if node.kind() == "if_statement" {
            if node.child_by_field_name("alternative").is_some() {
                has_else = true;
                else_text.get_or_insert_with(|| node_text(node, source).trim().to_string());
            } else if returns(node.child_by_field_name("consequence")) {
                has_guard = true;
                guard_text.get_or_insert_with(|| node_text(node, source).trim().to_string());
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            stack.push(child);
        }
    }
    Flow {
        has_else,
        has_guard,
        sample: else_text.or(guard_text).unwrap_or_default(),
    }
}

fn returns(consequence: Option<Node<'_>>) -> bool {
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

fn grammar_of(tree: &Tree) -> SupportedLanguage {
    let actual = tree.language();
    if *actual == SupportedLanguage::TypeScript.language() {
        SupportedLanguage::TypeScript
    } else {
        // JavaScript is parsed with the TSX grammar, so it shares that query.
        SupportedLanguage::Tsx
    }
}

fn for_each_match<'a>(
    query: &'static ScriptQuery,
    tree: &'a Tree,
    source: &'a str,
    mut on_match: impl FnMut(&[&str], &[(usize, Node<'a>)]) -> Option<()>,
) {
    let Some(compiled) = query.get(grammar_of(tree)) else {
        return;
    };
    let names = compiled.capture_names();
    for group in crate::ast::matches(compiled, tree, source) {
        on_match(names, &group);
    }
}

fn node_of<'a>(name: &str, names: &[&str], nodes: &[(usize, Node<'a>)]) -> Option<Node<'a>> {
    let index = names.iter().position(|candidate| *candidate == name)?;
    nodes
        .iter()
        .find(|(capture, _)| *capture == index)
        .map(|(_, node)| *node)
}

fn text_of<'a>(
    name: &str,
    names: &[&str],
    nodes: &[(usize, Node<'a>)],
    source: &'a str,
) -> Option<&'a str> {
    let index = names.iter().position(|candidate| *candidate == name)?;
    nodes
        .iter()
        .find(|(capture, _)| *capture == index)
        .map(|(_, node)| node_text(*node, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entity_file_is_scoped_to_entity_files_under_its_top_directory() {
        assert_eq!(
            scope_for_path(Path::new("src/users/user.entity.ts")),
            "src/**/*.entity.ts"
        );
        assert_eq!(
            scope_for_path(Path::new("src\\users\\user.entity.ts")),
            "src/**/*.entity.ts"
        );
        assert_eq!(
            scope_for_path(Path::new("user.entity.ts")),
            "**/*.entity.ts"
        );
        assert_eq!(scope_for_path(Path::new("src/main.rs")), "src/**/*.rs");
    }

    #[test]
    fn analyze_scrubs_a_secret_inside_a_snippet_and_keeps_the_identifier() {
        let before = "@Entity(\"AKIAIOSFODNN7EXAMPLE\")\nexport class User {}\n";
        let after = "@AutoEntity(\"AKIAIOSFODNN7EXAMPLE\")\nexport class User {}\n";
        let found = analyze(before, after, Path::new("src/users/user.entity.ts")).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let anti = found[0].anti_pattern.as_deref().unwrap();
        assert!(!anti.contains("AKIAIOSFODNN7EXAMPLE"), "{anti}");
        assert!(anti.contains("Entity"), "{anti}");
        assert!(anti.contains("<REDACTED_SECRET>"), "{anti}");
        assert_eq!(found[0].scope, "src/**/*.entity.ts");
    }
}
