//! Concrete structural edits between a before and an after buffer.
//!
//! Every snippet this module produces passes through [`super::scrub`] inside
//! [`analyze`], and is then cut to [`MAX_SNIPPET_LINES`] lines and
//! [`MAX_SNIPPET_BYTES`] bytes. Detectors return the raw node text; they do not
//! redact or trim it themselves, so a new detector cannot skip either step.
//!
//! An edit is ignored when a syntax error overlaps the lines it changed. A
//! half-typed buffer parses into error-recovery trees whose node shapes are
//! guesses, and a rule learned from a guess is a wrong rule. An error
//! elsewhere in the file does not block an edit that is clear of it.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::ast::{
    decorator_name, import_locals, is_custom_error, is_framework_decorator, is_package_specifier,
    node_text, parse, unquote, Family, ScriptQuery, SupportedLanguage,
};
use crate::config::Paths;
use crate::models::{RuleCategory, RuleSource};
use crate::storage::{validate_scope, Observation};

use super::scrub;

/// Longest snippet, in lines, stored as an anti or preferred pattern.
pub const MAX_SNIPPET_LINES: usize = 12;
/// Longest snippet, in bytes, stored as an anti or preferred pattern.
pub const MAX_SNIPPET_BYTES: usize = 400;
/// Last line of a truncated snippet. It is a comment, and comments never
/// match in [`super::contradictions`], so a truncated anti-pattern can never
/// decay a rule by matching only its first few lines.
const TRUNCATED: &str = "// …";

/// Instruction the logger-replacement detector emits.
pub const PREFER_LOGGER: &str = "Prefer a logger service over `console.log` and `console.error`.";

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
    let detectors = match lang.family() {
        _ if !lang.has_analyzers() => return Ok(Vec::new()),
        Family::Script => DETECTORS,
        Family::Python => super::python::DETECTORS,
        Family::Rust => super::rust::DETECTORS,
        Family::Go => return Ok(Vec::new()),
    };
    let (Some(before_tree), Some(after_tree)) = (parse(lang, before), parse(lang, after)) else {
        return Ok(Vec::new());
    };

    let (removed, added) = changed_lines(before, after);
    if touches_error(&before_tree, &removed) || touches_error(&after_tree, &added) {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for detector in detectors {
        let Some(observation) = detector.detect(&before_tree, before, &after_tree, after, path)
        else {
            continue;
        };
        if !grounded(&observation, before, after, &removed, &added) {
            continue;
        }
        found.push(prepare_snippets(observation, lang));
    }
    Ok(found)
}

pub(super) trait Detector: Sync {
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
        let before_imports = imports(before_tree, before_src);
        let after_imports = imports(after_tree, after_src);
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
            // Only a framework decorator giving way to a project-specific one
            // is a taste. `@Get` to `@Post` or `@IsString` to `@IsNumber`
            // changes what the code does, and learning it as a preference
            // would rewrite every other file in scope.
            if !from_framework(&swap.old_name, &before_imports)
                || from_framework(&swap.new_name, &after_imports)
            {
                continue;
            }
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
            // Only the alias-versus-deep-relative choice the scanner measures is
            // a taste. `@/users/x` becoming `./x` is a sibling import, which
            // says nothing about deep paths, and learning it as "prefer
            // relative" would contradict a scanned alias rule.
            let instruction = match (from, to) {
                (ImportClass::DeepRelative, ImportClass::Alias) => crate::ast::PREFER_PATH_ALIASES,
                (ImportClass::Alias, ImportClass::DeepRelative) => crate::ast::PREFER_DEEP_RELATIVE,
                _ => continue,
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
            // An unmatched console call (deleted outright) is no reason to stop
            // looking at the ones that were replaced.
            let Some(logger) = after
                .iter()
                .find(|call| call.kind == CallKind::Logger && call.args == console.args)
            else {
                continue;
            };
            return Some(observation(
                RuleCategory::CodeStyle,
                path,
                PREFER_LOGGER,
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
        let after_imports = imports(after_tree, after_src);
        for (args, before_throw) in &before {
            let Some(after_throw) = after.get(args) else {
                continue;
            };
            if before_throw.constructor == after_throw.constructor {
                continue;
            }
            // `Error` becoming `NotFoundException` or a class from some package
            // is a different built-in, not a project's own error class.
            if !is_bare_error(&before_throw.constructor)
                || !is_custom_error(&after_throw.constructor)
                || from_package(&after_throw.constructor, &after_imports)
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

pub(super) fn observation(
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

/// Scrub, then trim. Scrubbing sees the whole snippet, so a cut can never
/// leave behind a fragment of a secret the patterns no longer recognise.
fn prepare_snippets(mut observation: Observation, lang: SupportedLanguage) -> Observation {
    for snippet in [
        observation.anti_pattern.as_mut(),
        observation.preferred_pattern.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        *snippet = truncate_snippet(&scrub(snippet, lang));
    }
    observation
}

/// `snippet` cut at a line boundary to fit both limits, ending in
/// [`TRUNCATED`] when anything was dropped.
fn truncate_snippet(snippet: &str) -> String {
    if snippet.len() <= MAX_SNIPPET_BYTES && snippet.lines().count() <= MAX_SNIPPET_LINES {
        return snippet.to_string();
    }
    let mut kept = String::new();
    for line in snippet.lines().take(MAX_SNIPPET_LINES) {
        if kept.is_empty() && line.len() > MAX_SNIPPET_BYTES {
            let mut cut = MAX_SNIPPET_BYTES;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            kept.push_str(&line[..cut]);
            break;
        }
        if !kept.is_empty() && kept.len() + 1 + line.len() > MAX_SNIPPET_BYTES {
            break;
        }
        if !kept.is_empty() {
            kept.push('\n');
        }
        kept.push_str(line);
    }
    format!("{}\n{TRUNCATED}", kept.trim_end())
}

/// Whether an `ERROR` or `MISSING` node in `tree` covers any of `lines`.
fn touches_error(tree: &Tree, lines: &HashSet<usize>) -> bool {
    let root = tree.root_node();
    if !root.has_error() {
        return false;
    }
    let mut rows = Vec::new();
    collect_error_rows(root, &mut rows);
    rows.iter()
        .any(|(start, end)| (*start..=*end).any(|line| lines.contains(&line)))
}

fn collect_error_rows(node: Node<'_>, rows: &mut Vec<(usize, usize)>) {
    if node.is_error() || node.is_missing() {
        rows.push((node.start_position().row, node.end_position().row));
        return;
    }
    if !node.has_error() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_error_rows(child, rows);
    }
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
    let pattern = file_pattern(file, path);
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

fn file_pattern(file: &str, path: &Path) -> String {
    if let Some(pattern) = SupportedLanguage::from_path(path)
        .and_then(|lang| crate::ast::test_file_pattern(file, lang))
    {
        return pattern;
    }
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

/// [`analyze`], with each scope narrowed to the monorepo package that holds
/// `path`: `apps/api/src/x.service.ts` gives `apps/api/**/*.service.ts`
/// rather than `apps/**/*.service.ts`, which would cover every app. A file in
/// the root package keeps the scope [`analyze`] gives it.
pub fn analyze_in(
    paths: &Paths,
    before: &str,
    after: &str,
    path: &Path,
) -> crate::error::Result<Vec<Observation>> {
    let mut observations = analyze(before, after, path)?;
    if let Some(package) = scope_root(paths, path) {
        for observation in &mut observations {
            observation.scope = package_scope(&observation.scope, &package);
        }
    }
    Ok(observations)
}

/// The directory an observation from `path` is narrowed to: its monorepo
/// package, or that package's `tests/` when `path` is one of a crate's
/// integration tests, which are their own role as `*_test.go` files are.
fn scope_root(paths: &Paths, path: &Path) -> Option<String> {
    let package = paths.package_dir(path)?;
    let in_tests = SupportedLanguage::from_path(path) == Some(SupportedLanguage::Rust)
        && path
            .strip_prefix(&package)
            .is_ok_and(|inner| inner.starts_with("tests"));
    Some(if in_tests {
        format!("{package}/tests")
    } else {
        package
    })
}

/// `scope`'s file pattern under `package`, or `scope` unchanged when the
/// package path cannot be written as a glob.
fn package_scope(scope: &str, package: &str) -> String {
    let pattern = scope
        .rsplit_once("**/")
        .map_or(scope, |(_, pattern)| pattern);
    let narrowed = format!("{package}/**/{pattern}");
    if glob_meta(package) || validate_scope(&narrowed).is_err() {
        return scope.to_string();
    }
    narrowed
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

/// A decorator that comes from a framework rather than from this project:
/// imported from a package, or a name the scanner knows that the project did
/// not import from its own files.
fn from_framework(name: &str, imports: &HashMap<String, ImportBinding>) -> bool {
    match imports.get(name) {
        Some(binding) => is_package_specifier(&binding.path),
        None => is_framework_decorator(name),
    }
}

fn from_package(name: &str, imports: &HashMap<String, ImportBinding>) -> bool {
    imports
        .get(name)
        .is_some_and(|binding| is_package_specifier(&binding.path))
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
    /// `../../` or deeper, the side the scanner counts against aliases.
    DeepRelative,
    /// `./x` or `../x`.
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

fn import_class(path: &str) -> Option<ImportClass> {
    if path.starts_with("@/") || path.starts_with("~/") {
        Some(ImportClass::Alias)
    } else if path.contains("../..") {
        Some(ImportClass::DeepRelative)
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
    let mut found = Vec::new();
    for_each_match(&TYPES, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?;
        let decl = node_of("decl", names, nodes)?;
        let text = node_text(decl, source).trim().to_string();
        let kind = if decl.kind() == "interface_declaration" {
            TypeKind::Interface
        } else {
            TypeKind::Alias
        };
        found.push((qualified(decl, source, name), TypeDecl { kind, text }));
        Some(())
    });
    unique(found)
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
    let mut found = Vec::new();
    for_each_match(&FUNCTIONS, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?;
        let decl = node_of("decl", names, nodes)?;
        let kind = if decl.kind() == "function_declaration" {
            FnKind::Declaration
        } else {
            FnKind::Arrow
        };
        found.push((
            qualified(decl, source, name),
            FnDecl {
                kind,
                text: node_text(decl, source).trim().to_string(),
            },
        ));
        Some(())
    });
    unique(found)
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

/// Throws keyed by the function they sit in plus their arguments, so
/// `throw new X()` in two different functions cannot pair with each other.
fn throws(tree: &Tree, source: &str) -> HashMap<String, Throw> {
    let mut found = Vec::new();
    for_each_match(&THROWS, tree, source, |names, nodes| {
        let constructor = text_of("constructor", names, nodes, source)?.to_string();
        let args = text_of("args", names, nodes, source)?;
        let throw = node_of("throw", names, nodes)?;
        let text = node_text(throw, source).trim().to_string();
        found.push((qualified(throw, source, args), Throw { constructor, text }));
        Some(())
    });
    unique(found)
}

struct Flow {
    has_else: bool,
    has_guard: bool,
    sample: String,
}

/// Function declarations, class methods and `const` arrows with a block body.
/// Most NestJS and Angular logic lives in methods, so leaving them out would
/// make the detector silent exactly where it matters.
static FLOWS: ScriptQuery = ScriptQuery::new(
    r#"
    (function_declaration name: (identifier) @name body: (statement_block) @body) @fn
    (method_definition name: (_) @name body: (statement_block) @body) @fn
    (lexical_declaration
      "const"
      (variable_declarator
        name: (identifier) @name
        value: (arrow_function body: (statement_block) @body))) @fn
    "#,
);

fn flows(tree: &Tree, source: &str) -> HashMap<String, Flow> {
    let mut found = Vec::new();
    for_each_match(&FLOWS, tree, source, |names, nodes| {
        let name = text_of("name", names, nodes, source)?;
        let function = node_of("fn", names, nodes)?;
        let body = node_of("body", names, nodes)?;
        found.push((
            qualified(function, source, name),
            inspect_flow(body, source),
        ));
        Some(())
    });
    unique(found)
}

/// `name` prefixed with the named scopes around `node`: `UserService.load`,
/// `outer.inner`. Two same-named functions in different scopes must not
/// pair with each other across a before and after buffer.
fn qualified(node: Node<'_>, source: &str, name: &str) -> String {
    let mut parts = vec![name.to_string()];
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if let Some(scope) = scope_name(ancestor, source) {
            parts.push(scope.to_string());
        }
        current = ancestor.parent();
    }
    parts.reverse();
    parts.join(".")
}

fn scope_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    match node.kind() {
        "class_declaration"
        | "abstract_class_declaration"
        | "class"
        | "function_declaration"
        | "generator_function_declaration"
        | "method_definition"
        | "interface_declaration"
        | "internal_module" => node
            .child_by_field_name("name")
            .map(|name| node_text(name, source)),
        "variable_declarator" => {
            let value = node.child_by_field_name("value")?;
            if matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "function" | "class"
            ) {
                node.child_by_field_name("name")
                    .map(|name| node_text(name, source))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Keep only keys that occur once. A key that occurs twice (overloads, or two
/// identical throws in one function) cannot be paired across buffers with any
/// confidence, so it is dropped rather than letting the last one win.
fn unique<T>(pairs: Vec<(String, T)>) -> HashMap<String, T> {
    let mut slots: HashMap<String, Option<T>> = HashMap::new();
    for (key, value) in pairs {
        slots
            .entry(key)
            .and_modify(|slot| *slot = None)
            .or_insert(Some(value));
    }
    slots
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect()
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
    fn a_test_file_is_scoped_to_test_files_of_its_language() {
        assert_eq!(
            scope_for_path(Path::new("pkg/users/users_test.go")),
            "pkg/**/*_test.go"
        );
        assert_eq!(
            scope_for_path(Path::new("app/tests/test_users.py")),
            "app/**/test_*.py"
        );
        assert_eq!(
            scope_for_path(Path::new("app/users_test.py")),
            "app/**/*_test.py"
        );
        assert_eq!(scope_for_path(Path::new("tests/cli.rs")), "tests/**/*.rs");
        assert_eq!(
            scope_for_path(Path::new("pkg/users/users.go")),
            "pkg/**/*.go"
        );
    }

    #[test]
    fn a_crates_integration_tests_keep_their_own_scope_inside_the_package() {
        let dir = tempfile::tempdir().unwrap();
        for package in ["crates/core", "web"] {
            std::fs::create_dir_all(dir.path().join(package)).unwrap();
        }
        std::fs::write(dir.path().join("crates/core/Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(dir.path().join("web/package.json"), "{}\n").unwrap();
        let paths = Paths::for_root(dir.path());
        let root = |path: &str| scope_root(&paths, Path::new(path));
        assert_eq!(
            root("crates/core/tests/scan.rs").as_deref(),
            Some("crates/core/tests")
        );
        assert_eq!(
            root("crates/core/src/lib.rs").as_deref(),
            Some("crates/core")
        );
        assert_eq!(
            root("web/tests/login.ts").as_deref(),
            Some("web"),
            "a TypeScript tests/ directory is not a Rust test target"
        );
        assert_eq!(root("loose/file.rs"), None);
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

    #[test]
    fn a_short_snippet_is_stored_whole() {
        let snippet = "function load() {\n  return 1;\n}";
        assert_eq!(truncate_snippet(snippet), snippet);
    }

    #[test]
    fn a_long_snippet_is_cut_at_a_line_and_marked() {
        let body: String = (0..40).map(|i| format!("  step{i}();\n")).collect();
        let snippet = format!("function load() {{\n{body}}}");
        let cut = truncate_snippet(&snippet);
        assert!(cut.lines().count() <= MAX_SNIPPET_LINES + 1, "{cut}");
        assert!(
            cut.len() <= MAX_SNIPPET_BYTES + TRUNCATED.len() + 1,
            "{cut}"
        );
        assert!(cut.starts_with("function load() {\n  step0();"), "{cut}");
        assert!(cut.ends_with(TRUNCATED), "{cut}");
        assert!(snippet.starts_with(cut.trim_end_matches(TRUNCATED).trim_end()));
    }

    #[test]
    fn one_enormous_line_is_cut_on_a_character_boundary() {
        let snippet = format!("const s = \"{}\";", "é".repeat(400));
        let cut = truncate_snippet(&snippet);
        assert!(cut.ends_with(TRUNCATED), "{cut}");
        assert!(cut.len() <= MAX_SNIPPET_BYTES + TRUNCATED.len() + 1);
    }

    #[test]
    fn a_whole_function_swap_stores_a_bounded_snippet() {
        let body: String = (0..60).map(|i| format!("  step{i}();\n")).collect();
        let before = format!("function load() {{\n{body}}}\n");
        let after = format!("const load = () => {{\n{body}}};\n");
        let found = analyze(&before, &after, Path::new("src/app.ts")).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        for snippet in [&found[0].anti_pattern, &found[0].preferred_pattern] {
            let snippet = snippet.as_deref().unwrap();
            assert!(
                snippet.len() <= MAX_SNIPPET_BYTES + TRUNCATED.len() + 1,
                "{snippet}"
            );
            assert!(snippet.ends_with(TRUNCATED), "{snippet}");
        }
    }

    #[test]
    fn a_secret_straddling_the_cut_is_scrubbed_before_it_could_be_split() {
        // The key starts a few bytes before the cut. Trimming first would keep
        // `AKIAIOSF`, which no pattern recognises any more.
        let filler = "x".repeat(382);
        let before =
            format!("@Entity(\"{filler} AKIAIOSFODNN7EXAMPLE\")\nexport class User {{}}\n");
        let after = before.replacen("@Entity", "@AutoEntity", 1);
        let found = analyze(&before, &after, Path::new("src/users/user.entity.ts")).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let anti = found[0].anti_pattern.as_deref().unwrap();
        assert!(anti.ends_with(TRUNCATED), "{anti}");
        assert!(!anti.contains("AKIA"), "{anti}");
    }

    #[test]
    fn a_half_typed_edit_learns_nothing() {
        // Both buffers fire a detector when the error guard is removed: the
        // recovered tree still looks like a finished refactor.
        let cases = [
            (
                "function f(x) {\n  if (x) {\n    return 1;\n  } else {\n    return 2;\n  }\n}\n",
                "function f(x) {\n  if (!x) return 2;\n  return 1\n  if (\n}\n",
            ),
            (
                "function f() {\n  throw new Error(\"x\");\n}\n",
                "function f() {\n  throw new DomainError(\"x\");\n  const\n}\n",
            ),
        ];
        for (before, after) in cases {
            let found = analyze(before, after, Path::new("src/app.ts")).unwrap();
            assert!(found.is_empty(), "{after}: {found:?}");
        }
    }

    #[test]
    fn a_syntax_error_away_from_the_edit_does_not_block_it() {
        let broken = "\n\n\n\n\n\n\n\nconst broken = (;\n";
        let before = format!("function load() {{\n  return 1;\n}}\n{broken}");
        let after = format!("const load = () => {{\n  return 1;\n}};\n{broken}");
        let found = analyze(&before, &after, Path::new("src/app.ts")).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
