//! Python conventions.
//!
//! Where a name comes from decides what it says, as in [`super::arch`]. Python
//! has no path aliases, so "the project's own package" is read from the file's
//! path: an absolute import whose top module is one of the directories the
//! file lives in (`from myapp.users import User` inside `src/myapp/api.py`) is
//! first-party. Every other absolute import is a package or the standard
//! library, and counts toward neither import style.
//!
//! Each dimension only counts what a reasonable team could choose either way.
//! `ValueError` is not a bare exception, `@property` is not a framework
//! decorator, and `print()` in a command-line entry point is its output, not a
//! missing logger. Code under `if __name__ == "__main__":` is a script and is
//! skipped for every dimension.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use tree_sitter::{Node, Tree};

use super::{Count, PythonTally};
use crate::ast::query::node_text;

/// Where a name bound by an import comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A relative import or an absolute import of the project's own package.
    Project,
    /// The standard library, or `typing_extensions`, which stands in for it.
    Stdlib,
    /// Anything else: an installed package.
    Package,
}

/// How an import statement names its module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportStyle {
    /// `from .users import User`.
    Relative,
    /// `from myapp.users import User`, where `myapp` is the file's own package.
    FirstParty,
    /// The standard library or a package.
    External,
}

/// One import statement and the local names it binds.
#[derive(Debug, Clone)]
pub(crate) struct Import {
    pub(crate) style: ImportStyle,
    pub(crate) top: String,
    pub(crate) names: Vec<String>,
    pub(crate) statement: String,
}

/// What a file imports and defines, for deciding where a name comes from.
pub(crate) struct Module {
    pub(crate) imports: Vec<Import>,
    origins: HashMap<String, Origin>,
    /// Classes defined in the file that extend an exception class.
    exception_classes: HashSet<String>,
    /// Functions defined in the file, which can be used as decorators.
    functions: HashSet<String>,
    /// Top-level modules imported, such as `argparse` or `click`.
    modules: HashSet<String>,
}

impl Module {
    pub(crate) fn of(path: &Path, tree: &Tree, source: &str) -> Self {
        let roots = package_roots(path);
        let mut module = Module {
            imports: Vec::new(),
            origins: HashMap::new(),
            exception_classes: HashSet::new(),
            functions: HashSet::new(),
            modules: HashSet::new(),
        };
        for node in code_nodes(tree, source) {
            match node.kind() {
                "import_statement" | "import_from_statement" => {
                    if let Some(import) = read_import(node, source, &roots) {
                        let origin = match import.style {
                            ImportStyle::Relative | ImportStyle::FirstParty => Origin::Project,
                            ImportStyle::External if is_stdlib(&import.top) => Origin::Stdlib,
                            ImportStyle::External => Origin::Package,
                        };
                        module.modules.insert(import.top.clone());
                        for name in &import.names {
                            module.origins.insert(name.clone(), origin);
                        }
                        module.imports.push(import);
                    }
                }
                "class_definition" => {
                    if extends_exception(node, source) {
                        if let Some(name) = node.child_by_field_name("name") {
                            module
                                .exception_classes
                                .insert(node_text(name, source).to_string());
                        }
                    }
                }
                "function_definition" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        module.functions.insert(node_text(name, source).to_string());
                    }
                }
                _ => {}
            }
        }
        module
    }

    pub(crate) fn origin(&self, name: &str) -> Option<Origin> {
        self.origins.get(name).copied()
    }

    /// Whether raising `name` raises one of the project's own exceptions.
    ///
    /// Defined here or imported from the project. A name of unknown origin is
    /// not assumed to be the project's: it may come from a star import.
    pub(crate) fn is_project_exception(&self, name: &str) -> bool {
        let root = name.split('.').next().unwrap_or(name);
        if name.contains('.') {
            return self.origin(root) == Some(Origin::Project);
        }
        self.exception_classes.contains(name) || self.origin(name) == Some(Origin::Project)
    }

    /// Whether `print` here is the builtin, not `rich.print` or a local helper.
    fn print_is_builtin(&self) -> bool {
        !self.origins.contains_key("print") && !self.functions.contains("print")
    }

    /// Files whose `print()` is their output: entry points and command-line
    /// tools. Their logging dimension is not counted at all.
    fn writes_to_a_terminal(&self, path: &Path) -> bool {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let in_cli_dir = path.components().any(|part| {
            matches!(part, Component::Normal(name)
                if matches!(name.to_str(), Some("cli" | "commands" | "management" | "bin")))
        });
        matches!(file, "__main__.py" | "manage.py" | "setup.py" | "cli.py")
            || in_cli_dir
            || CLI_MODULES
                .iter()
                .any(|module| self.modules.contains(*module))
    }
}

/// Modules whose presence makes a file a command-line tool.
const CLI_MODULES: &[&str] = &["argparse", "click", "typer", "fire", "docopt", "optparse"];

/// Standard-library modules a project imports exceptions or decorators from.
/// Not the whole standard library: a module missing here is treated as a
/// package, which only matters for decorators, where it means "framework".
const STDLIB: &[&str] = &[
    "__future__",
    "abc",
    "argparse",
    "asyncio",
    "atexit",
    "builtins",
    "collections",
    "concurrent",
    "contextlib",
    "copy",
    "dataclasses",
    "datetime",
    "enum",
    "functools",
    "http",
    "importlib",
    "inspect",
    "io",
    "itertools",
    "json",
    "logging",
    "math",
    "operator",
    "os",
    "pathlib",
    "pickle",
    "random",
    "re",
    "shutil",
    "signal",
    "socket",
    "sqlite3",
    "string",
    "subprocess",
    "sys",
    "tempfile",
    "threading",
    "time",
    "types",
    "typing",
    "typing_extensions",
    "unittest",
    "urllib",
    "uuid",
    "warnings",
    "weakref",
];

fn is_stdlib(top: &str) -> bool {
    STDLIB.contains(&top)
}

/// The directories `path` lives in, any of which may be its package's name.
fn package_roots(path: &Path) -> HashSet<String> {
    path.parent()
        .into_iter()
        .flat_map(Path::components)
        .filter_map(|part| match part {
            Component::Normal(name) => name.to_str().map(str::to_string),
            _ => None,
        })
        .collect()
}

pub(crate) fn read_imports(path: &Path, tree: &Tree, source: &str) -> Vec<Import> {
    Module::of(path, tree, source).imports
}

fn read_import(node: Node<'_>, source: &str, roots: &HashSet<String>) -> Option<Import> {
    let statement = node_text(node, source).to_string();
    let mut names = Vec::new();
    let mut cursor = node.walk();
    if node.kind() == "import_from_statement" {
        let module = node.child_by_field_name("module_name")?;
        for name in node.children_by_field_name("name", &mut cursor) {
            names.push(bound_name(name, source, true)?);
        }
        if module.kind() == "relative_import" {
            return Some(Import {
                style: ImportStyle::Relative,
                top: String::new(),
                names,
                statement,
            });
        }
        let top = top_module(module, source);
        let style = if roots.contains(&top) {
            ImportStyle::FirstParty
        } else {
            ImportStyle::External
        };
        return Some(Import {
            style,
            top,
            names,
            statement,
        });
    }
    // `import a.b, c as d`. A statement that mixes a first-party module with
    // another is rare; the first module decides its style.
    let first = node.children_by_field_name("name", &mut cursor).next()?;
    let mut cursor = node.walk();
    for name in node.children_by_field_name("name", &mut cursor) {
        names.push(bound_name(name, source, false)?);
    }
    let module = if first.kind() == "aliased_import" {
        first.child_by_field_name("name")?
    } else {
        first
    };
    let top = top_module(module, source);
    let style = if roots.contains(&top) {
        ImportStyle::FirstParty
    } else {
        ImportStyle::External
    };
    Some(Import {
        style,
        top,
        names,
        statement,
    })
}

/// The local name an import binds. `from x import a.b` cannot happen, so a
/// dotted name in a `from` import is one identifier; `import a.b` binds `a`.
fn bound_name(node: Node<'_>, source: &str, from_import: bool) -> Option<String> {
    if node.kind() == "aliased_import" {
        return Some(node_text(node.child_by_field_name("alias")?, source).to_string());
    }
    let text = node_text(node, source);
    Some(if from_import {
        text.to_string()
    } else {
        text.split('.').next()?.to_string()
    })
}

fn top_module(dotted: Node<'_>, source: &str) -> String {
    node_text(dotted, source)
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn extends_exception(class: Node<'_>, source: &str) -> bool {
    let Some(bases) = class.child_by_field_name("superclasses") else {
        return false;
    };
    let mut cursor = bases.walk();
    let found = bases.named_children(&mut cursor).any(|base| {
        let name = node_text(base, source);
        let last = name.rsplit('.').next().unwrap_or(name);
        last.ends_with("Error") || last.ends_with("Exception")
    });
    found
}

/// Every node outside `if __name__ == "__main__":` blocks, in document order.
pub(crate) fn code_nodes<'a>(tree: &'a Tree, source: &str) -> Vec<Node<'a>> {
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_main_guard(node, tree, source) {
            continue;
        }
        found.push(node);
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    found
}

fn is_main_guard(node: Node<'_>, tree: &Tree, source: &str) -> bool {
    if node.kind() != "if_statement" || node.parent() != Some(tree.root_node()) {
        return false;
    }
    let Some(condition) = node.child_by_field_name("condition") else {
        return false;
    };
    if condition.kind() != "comparison_operator" {
        return false;
    }
    let mut cursor = condition.walk();
    let parts: Vec<_> = condition.named_children(&mut cursor).collect();
    let text = |node: &Node<'_>| node_text(*node, source);
    parts.len() == 2
        && parts.iter().any(|part| text(part) == "__name__")
        && parts
            .iter()
            .any(|part| matches!(text(part), "\"__main__\"" | "'__main__'"))
}

/// Count every Python dimension in one file.
pub(super) fn tally(path: &Path, tree: &Tree, source: &str) -> PythonTally {
    let module = Module::of(path, tree, source);
    let nodes = code_nodes(tree, source);
    PythonTally {
        imports: imports(&module),
        errors: errors(&module, &nodes, source),
        logging: if module.writes_to_a_terminal(path) {
            Count::default()
        } else {
            logging(&module, &nodes, source)
        },
        control_flow: control_flow(&nodes),
        annotations: annotations(&nodes, source),
        decorators: decorators(&module, &nodes, source),
    }
}

fn imports(module: &Module) -> Count {
    let absolute = module
        .imports
        .iter()
        .filter(|import| import.style == ImportStyle::FirstParty)
        .count();
    let relative = module
        .imports
        .iter()
        .filter(|import| import.style == ImportStyle::Relative)
        .count();
    Count::from_sides(absolute as u32, relative as u32)
}

/// What a `raise` raises, by name: `Exception`, `errors.NotFound`. A bare
/// `raise` and `raise error` from a variable have no class to name.
pub(crate) fn raised_class<'a>(raise: Node<'_>, source: &'a str) -> Option<&'a str> {
    let mut cursor = raise.walk();
    let value = raise.named_children(&mut cursor).next()?;
    let class = match value.kind() {
        "call" => value.child_by_field_name("function")?,
        _ => value,
    };
    matches!(class.kind(), "identifier" | "attribute").then(|| node_text(class, source))
}

/// Raising one of these says nothing about a project's own exception classes.
pub(crate) fn is_bare_exception(name: &str) -> bool {
    name == "Exception"
}

fn errors(module: &Module, nodes: &[Node<'_>], source: &str) -> Count {
    let mut custom = module.exception_classes.len() as u32;
    let mut bare = 0u32;
    for node in nodes.iter().filter(|node| node.kind() == "raise_statement") {
        let Some(class) = raised_class(*node, source) else {
            continue;
        };
        if is_bare_exception(class) && module.origin("Exception").is_none() {
            bare += 1;
        } else if module.is_project_exception(class) {
            custom += 1;
        }
    }
    Count::from_sides(custom, bare)
}

const LOG_METHODS: &[&str] = &[
    "debug",
    "info",
    "warning",
    "warn",
    "error",
    "exception",
    "critical",
];

/// `logger.info(…)`, `self.log.warning(…)` or `logging.error(…)`.
pub(crate) fn is_logger_call(call: Node<'_>, source: &str) -> bool {
    let Some(function) = call.child_by_field_name("function") else {
        return false;
    };
    if function.kind() != "attribute" {
        return false;
    }
    let (Some(object), Some(method)) = (
        function.child_by_field_name("object"),
        function.child_by_field_name("attribute"),
    ) else {
        return false;
    };
    if !LOG_METHODS.contains(&node_text(method, source)) {
        return false;
    }
    let receiver = node_text(object, source);
    let last = receiver.rsplit('.').next().unwrap_or(receiver);
    matches!(
        last.trim_start_matches('_').to_ascii_lowercase().as_str(),
        "logger" | "log" | "logging"
    )
}

/// A call to the builtin `print`.
pub(crate) fn is_print_call(call: Node<'_>, source: &str) -> bool {
    call.child_by_field_name("function")
        .is_some_and(|function| {
            function.kind() == "identifier" && node_text(function, source) == "print"
        })
}

fn logging(module: &Module, nodes: &[Node<'_>], source: &str) -> Count {
    let mut logger = 0u32;
    let mut print = 0u32;
    for call in nodes.iter().filter(|node| node.kind() == "call") {
        if is_logger_call(*call, source) {
            logger += 1;
        } else if is_print_call(*call, source) && module.print_is_builtin() {
            print += 1;
        }
    }
    Count::from_sides(logger, print)
}

/// Whether a function body has an early-return guard and whether it has an
/// `if … else`. Nested functions and classes are their own bodies.
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
    if node.kind() == "if_statement" {
        let mut cursor = node.walk();
        let alternatives: Vec<_> = node
            .children_by_field_name("alternative", &mut cursor)
            .collect();
        if alternatives.iter().any(|alt| alt.kind() == "else_clause") {
            flow.has_else = true;
        } else if alternatives.is_empty() && exits(node.child_by_field_name("consequence")) {
            flow.has_guard = true;
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(
            child.kind(),
            "function_definition" | "class_definition" | "lambda"
        ) {
            continue;
        }
        visit_flow(child, flow);
    }
}

/// A block made only of `return` and `raise`.
fn exits(block: Option<Node<'_>>) -> bool {
    let Some(block) = block else {
        return false;
    };
    let mut cursor = block.walk();
    let statements: Vec<_> = block
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .collect();
    !statements.is_empty()
        && statements
            .iter()
            .all(|statement| matches!(statement.kind(), "return_statement" | "raise_statement"))
}

fn control_flow(nodes: &[Node<'_>]) -> Count {
    let mut guards = 0u32;
    let mut nested = 0u32;
    for function in nodes
        .iter()
        .filter(|node| node.kind() == "function_definition")
    {
        let flow = flow_of(*function);
        guards += u32::from(flow.has_guard);
        nested += u32::from(flow.has_else);
    }
    Count::from_sides(guards, nested)
}

/// Whether a signature is fully annotated, fully unannotated, or neither.
fn signature(function: Node<'_>, source: &str) -> Option<bool> {
    let name = function
        .child_by_field_name("name")
        .map(|name| node_text(name, source))
        .unwrap_or("");
    let params = function.child_by_field_name("parameters")?;
    let mut cursor = params.walk();
    let mut typed = 0usize;
    let mut untyped = 0usize;
    for (position, param) in params.named_children(&mut cursor).enumerate() {
        match param.kind() {
            "typed_parameter" | "typed_default_parameter" => typed += 1,
            "identifier" if position == 0 && matches!(node_text(param, source), "self" | "cls") => {
            }
            "identifier"
            | "default_parameter"
            | "list_splat_pattern"
            | "dictionary_splat_pattern" => untyped += 1,
            // `*` and `/` separators, comments.
            _ => {}
        }
    }
    let returns = function.child_by_field_name("return_type").is_some();
    let init = name == "__init__";
    if untyped == 0 && (returns || (init && typed > 0)) {
        return Some(true);
    }
    if typed == 0 && !returns && !(init && untyped == 0) {
        return Some(false);
    }
    None
}

fn annotations(nodes: &[Node<'_>], source: &str) -> Count {
    let mut annotated = 0u32;
    let mut bare = 0u32;
    for function in nodes
        .iter()
        .filter(|node| node.kind() == "function_definition")
    {
        match signature(*function, source) {
            Some(true) => annotated += 1,
            Some(false) => bare += 1,
            None => {}
        }
    }
    Count::from_sides(annotated, bare)
}

/// The name a decorator is looked up by: `login_required` for
/// `@login_required`, `pytest` for `@pytest.fixture(scope="module")`.
pub(crate) fn decorator_root<'a>(decorator: Node<'_>, source: &'a str) -> Option<&'a str> {
    let mut cursor = decorator.walk();
    let mut expression = decorator.named_children(&mut cursor).next()?;
    if expression.kind() == "call" {
        expression = expression.child_by_field_name("function")?;
    }
    let text = node_text(expression, source);
    // `@name.setter` belongs to `@property`.
    if text.ends_with(".setter") || text.ends_with(".getter") || text.ends_with(".deleter") {
        return None;
    }
    matches!(expression.kind(), "identifier" | "attribute")
        .then(|| text.split('.').next().unwrap_or(text))
}

/// `Some(true)` for a project decorator, `Some(false)` for a framework one,
/// `None` for a language decorator or one of unknown origin.
///
/// Origin alone decides. Builtins such as `@property` are never imported, and
/// `@dataclass` or `@lru_cache` come from the standard library, so both say
/// nothing; a name list would also catch a project's own `cache` module.
pub(crate) fn decorator_is_project(module: &Module, root: &str) -> Option<bool> {
    match module.origin(root) {
        Some(Origin::Project) => Some(true),
        Some(Origin::Package) => Some(false),
        Some(Origin::Stdlib) => None,
        None if module.functions.contains(root) => Some(true),
        None => None,
    }
}

fn decorators(module: &Module, nodes: &[Node<'_>], source: &str) -> Count {
    let mut project = 0u32;
    let mut framework = 0u32;
    for decorator in nodes.iter().filter(|node| node.kind() == "decorator") {
        let Some(root) = decorator_root(*decorator, source) else {
            continue;
        };
        match decorator_is_project(module, root) {
            Some(true) => project += 1,
            Some(false) => framework += 1,
            None => {}
        }
    }
    Count::from_sides(project, framework)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, SupportedLanguage};

    fn tally_at(path: &str, source: &str) -> PythonTally {
        let tree = parse(SupportedLanguage::Python, source).unwrap();
        tally(Path::new(path), &tree, source)
    }

    fn tally_of(source: &str) -> PythonTally {
        tally_at("src/myapp/users/service.py", source)
    }

    #[test]
    fn first_party_absolute_imports_count_against_relative_ones() {
        let unanimous = tally_of(
            "from myapp.models import User\nimport myapp.db\nfrom myapp.users.repo import Repo as R\n",
        );
        assert_eq!(unanimous.imports, Count::from_sides(3, 0));

        let mixed =
            tally_of("from myapp.models import User\nfrom .repo import Repo\nfrom .. import db\n");
        assert_eq!(mixed.imports, Count::from_sides(1, 2));

        let nothing = tally_of(
            "import os\nfrom typing import Any\nfrom django.db import models\nimport requests\n",
        );
        assert_eq!(
            nothing.imports,
            Count::default(),
            "packages are neither style"
        );
    }

    #[test]
    fn a_files_own_directories_decide_what_is_first_party() {
        let source = "from users.models import User\n";
        assert_eq!(
            tally_at("users/views.py", source).imports,
            Count::from_sides(1, 0)
        );
        assert_eq!(
            tally_at("orders/views.py", source).imports,
            Count::default()
        );
    }

    #[test]
    fn project_exceptions_count_against_bare_exception() {
        let unanimous = tally_of(
            "from .errors import NotFound\n\
             class Conflict(AppError):\n    pass\n\
             def f():\n    raise NotFound('x')\n\
             def g():\n    raise Conflict\n",
        );
        assert_eq!(unanimous.errors, Count::from_sides(3, 0));

        let mixed = tally_of(
            "from myapp import errors\n\
             def f():\n    raise errors.NotFound('x')\n\
             def g():\n    raise Exception('y')\n",
        );
        assert_eq!(mixed.errors, Count::from_sides(1, 1));

        let nothing = tally_of(
            "from fastapi import HTTPException\n\
             def f(x):\n    raise ValueError(x)\n\
             def g():\n    raise HTTPException(404)\n\
             def h(e):\n    raise e\n\
             def i():\n    raise\n\
             def j():\n    raise Unknown('z')\n",
        );
        assert_eq!(
            nothing.errors,
            Count::default(),
            "builtins, packages, variables and unknown names say nothing"
        );
    }

    #[test]
    fn logger_calls_count_against_print() {
        let unanimous = tally_of(
            "import logging\nlogger = logging.getLogger(__name__)\n\
             def f():\n    logger.info('a')\n    self._log.warning('b')\n    logging.error('c')\n",
        );
        assert_eq!(unanimous.logging, Count::from_sides(3, 0));

        let mixed = tally_of("def f():\n    logger.debug('a')\n    print('b')\n    print('c')\n");
        assert_eq!(mixed.logging, Count::from_sides(1, 2));

        let nothing = tally_of(
            "from rich import print\ndef f():\n    print('styled')\n    console.log('x')\n",
        );
        assert_eq!(
            nothing.logging,
            Count::default(),
            "rich.print is not the builtin"
        );
    }

    #[test]
    fn command_line_tools_and_main_blocks_do_not_count_print() {
        let source = "def f():\n    print('result')\n";
        for path in [
            "src/myapp/cli.py",
            "src/myapp/__main__.py",
            "src/myapp/cli/run.py",
            "manage.py",
        ] {
            assert_eq!(tally_at(path, source).logging, Count::default(), "{path}");
        }
        let with_argparse = tally_of("import argparse\ndef f():\n    print('result')\n");
        assert_eq!(with_argparse.logging, Count::default());

        let main_block = tally_of(
            "def f():\n    logger.info('x')\n\
             if __name__ == \"__main__\":\n    print('ran')\n    raise Exception('stop')\n",
        );
        assert_eq!(main_block.logging, Count::from_sides(1, 0));
        assert_eq!(main_block.errors, Count::default());
    }

    #[test]
    fn guard_clauses_count_against_else_branches() {
        let unanimous = tally_of(
            "def f(x):\n    if not x:\n        return None\n    return x\n\
             def g(x):\n    if x is None:\n        raise NotFound()\n    return x\n",
        );
        assert_eq!(unanimous.control_flow, Count::from_sides(2, 0));

        let mixed = tally_of(
            "def f(x):\n    if not x:\n        return None\n    return x\n\
             def g(x):\n    if x:\n        y = 1\n    else:\n        y = 2\n    return y\n",
        );
        assert_eq!(mixed.control_flow, Count::from_sides(1, 1));

        let nothing = tally_of(
            "def f(xs):\n    for x in xs:\n        pass\n    else:\n        pass\n\
             def g(x):\n    if x:\n        y = 1\n    elif x is None:\n        y = 2\n    return y\n\
             def h(x):\n    try:\n        pass\n    except E:\n        pass\n    else:\n        pass\n",
        );
        assert_eq!(
            nothing.control_flow,
            Count::default(),
            "for-else, try-else and elif without else are not nested if/else"
        );
    }

    #[test]
    fn a_nested_function_is_its_own_body() {
        let got = tally_of(
            "def outer(x):\n    def inner(y):\n        if y:\n            a = 1\n        else:\n            a = 2\n        return a\n    return inner\n",
        );
        assert_eq!(got.control_flow, Count::from_sides(0, 1));
    }

    #[test]
    fn annotated_signatures_count_against_unannotated_ones() {
        let unanimous = tally_of(
            "def f(a: int, *args: str, b: int = 1, **kw: Any) -> int:\n    return a\n\
             class C:\n    def __init__(self, a: int):\n        pass\n    def m(self) -> None:\n        pass\n",
        );
        assert_eq!(unanimous.annotations, Count::from_sides(3, 0));

        let mixed = tally_of(
            "def f(a: int) -> int:\n    return a\ndef g(a, b=1):\n    return a\ndef h():\n    pass\n",
        );
        assert_eq!(mixed.annotations, Count::from_sides(1, 2));

        let nothing = tally_of(
            "def partial(a: int, b):\n    return a\ndef ret_only(a) -> int:\n    return a\n\
             class C:\n    def __init__(self):\n        pass\n",
        );
        assert_eq!(
            nothing.annotations,
            Count::default(),
            "half-annotated is neither"
        );
    }

    #[test]
    fn project_decorators_count_against_framework_ones() {
        let unanimous = tally_of(
            "from .auth import login_required\nfrom myapp import cache\n\
             def traced(f):\n    return f\n\
             @login_required\n@cache.memoize(60)\n@traced\ndef view():\n    pass\n",
        );
        assert_eq!(unanimous.decorators, Count::from_sides(3, 0));

        let mixed = tally_of(
            "import pytest\nfrom .auth import login_required\n\
             @pytest.fixture(scope='module')\n@login_required\ndef view():\n    pass\n",
        );
        assert_eq!(mixed.decorators, Count::from_sides(1, 1));

        let nothing = tally_of(
            "from dataclasses import dataclass\nfrom functools import lru_cache\nimport abc\n\
             @dataclass\nclass C:\n    @property\n    def x(self):\n        return 1\n\
             @x.setter\n    def x(self, v):\n        pass\n\
             @abc.abstractmethod\n    def m(self):\n        pass\n\
             @lru_cache(maxsize=8)\ndef f():\n    pass\n\
             @app.route('/')\ndef index():\n    pass\n",
        );
        assert_eq!(
            nothing.decorators,
            Count::default(),
            "language decorators and locals of unknown origin say nothing"
        );
    }
}
