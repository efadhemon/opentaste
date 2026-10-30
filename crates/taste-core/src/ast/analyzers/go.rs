//! Go conventions.
//!
//! `gofmt` and `goimports` settle formatting and imports, so Go has three
//! dimensions, each narrowed to what is actually a choice:
//!
//! - **Errors.** A `return …, err` is only "bare" when `err` came straight
//!   from another package's call (`os.Open`, `json.Unmarshal`). An error from
//!   the project's own function or method may already carry its context.
//!   Wrapping means `fmt.Errorf` with a `%w` verb.
//! - **Logging.** `log/slog`, zap, zerolog and logrus against `fmt.Print*`
//!   and the standard `log.Print*`. A `package main`, anything under `cmd/`
//!   and `func main` print as their job and do not count.
//! - **Control flow.** `if err != nil { return … }` is the language's idiom,
//!   not a choice, so it does not count as a guard; other early returns do.

use std::collections::HashMap;
use std::path::{Component, Path};

use tree_sitter::{Node, Tree};

use super::{Count, GoTally};
use crate::ast::query::{node_text, unquote};

/// Count every Go dimension in one file.
pub(super) fn tally(path: &Path, tree: &Tree, source: &str) -> GoTally {
    let file = File::of(path, tree, source);
    let nodes = all_nodes(tree);
    GoTally {
        errors: errors(&file, &nodes, source),
        logging: if file.is_command {
            Count::default()
        } else {
            logging(&file, &nodes, source)
        },
        control_flow: control_flow(&nodes, source),
    }
}

/// Packages whose presence makes a file part of a command-line tool, whose
/// `fmt.Println` is its output.
const CLI_PACKAGES: &[&str] = &[
    "flag",
    "github.com/spf13/cobra",
    "github.com/urfave/cli",
    "github.com/alecthomas/kong",
    "github.com/peterbourgon/ff",
];

/// What a Go file imports, by local name.
pub(crate) struct File {
    /// Local name → import path.
    imports: HashMap<String, String>,
    /// `package main`, a file under `cmd/`, or one using a CLI package.
    pub(crate) is_command: bool,
}

impl File {
    pub(crate) fn of(path: &Path, tree: &Tree, source: &str) -> Self {
        let mut imports = HashMap::new();
        let mut package = "";
        for node in all_nodes(tree) {
            match node.kind() {
                "package_clause" => {
                    if let Some(name) = node.named_child(0) {
                        package = node_text(name, source);
                    }
                }
                "import_spec" => {
                    let Some(path) = node.child_by_field_name("path") else {
                        continue;
                    };
                    let path = unquote(node_text(path, source)).to_string();
                    let name = match node.child_by_field_name("name") {
                        Some(name) => node_text(name, source).to_string(),
                        None => default_name(&path),
                    };
                    if name != "_" && name != "." {
                        imports.insert(name, path);
                    }
                }
                _ => {}
            }
        }
        let in_cmd = path
            .components()
            .any(|part| matches!(part, Component::Normal(name) if name == "cmd"));
        let cli = imports.values().any(|path| {
            CLI_PACKAGES
                .iter()
                .any(|cli| path == cli || path.starts_with(&format!("{cli}/")))
        });
        File {
            imports,
            is_command: package == "main" || in_cmd || cli,
        }
    }

    /// The import path bound to `name`, if `name` is a package here.
    pub(crate) fn package(&self, name: &str) -> Option<&str> {
        self.imports.get(name).map(String::as_str)
    }
}

/// The name a package is used by when the import does not rename it: the last
/// path element, skipping a `/v2`-style major version. A name that differs
/// from its path (`go-redis` declaring `redis`) is not found, and calls on it
/// are simply not classified.
fn default_name(path: &str) -> String {
    let mut parts = path.rsplit('/');
    let last = parts.next().unwrap_or(path);
    let is_major = last.len() > 1
        && last.starts_with('v')
        && last[1..].bytes().all(|byte| byte.is_ascii_digit());
    if is_major {
        parts.next().unwrap_or(last).to_string()
    } else {
        last.to_string()
    }
}

pub(crate) fn all_nodes(tree: &Tree) -> Vec<Node<'_>> {
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        found.push(node);
        let mut cursor = node.walk();
        let children: Vec<_> = node.named_children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    found
}

/// The expression a `return` statement returns last: where Go puts the error.
pub(crate) fn last_returned(ret: Node<'_>) -> Option<Node<'_>> {
    let list = ret.named_child(0)?;
    if list.kind() == "expression_list" {
        list.named_child(list.named_child_count().checked_sub(1)? as u32)
    } else {
        Some(list)
    }
}

/// `fmt.Errorf("…%w…", …)`.
pub(crate) fn is_wrapping(file: &File, expression: Node<'_>, source: &str) -> bool {
    if expression.kind() != "call_expression" {
        return false;
    }
    let Some((package, function)) = package_call(file, expression, source) else {
        return false;
    };
    if package != "fmt" || function != "Errorf" {
        return false;
    }
    expression
        .child_by_field_name("arguments")
        .and_then(|args| args.named_child(0))
        .is_some_and(|format| node_text(format, source).contains("%w"))
}

/// `(import path, function)` when `call` calls a function of an imported package.
pub(crate) fn package_call<'a>(
    file: &'a File,
    call: Node<'_>,
    source: &'a str,
) -> Option<(&'a str, &'a str)> {
    let function = call.child_by_field_name("function")?;
    if function.kind() != "selector_expression" {
        return None;
    }
    let operand = function.child_by_field_name("operand")?;
    if operand.kind() != "identifier" {
        return None;
    }
    let package = file.package(node_text(operand, source))?;
    let field = function.child_by_field_name("field")?;
    Some((package, node_text(field, source)))
}

/// Whether `ret` returns, bare, an error that came straight from another
/// package's call: `if err := os.Remove(p); err != nil { return err }`.
pub(crate) fn is_bare_return(file: &File, ret: Node<'_>, source: &str) -> bool {
    let Some(value) = last_returned(ret).filter(|value| value.kind() == "identifier") else {
        return false;
    };
    let name = node_text(value, source);
    let Some(check) = enclosing_error_check(ret, name, source) else {
        return false;
    };
    origin_call(check, name, source).is_some_and(|call| package_call(file, call, source).is_some())
}

/// The `if <name> != nil` that `node` sits directly inside.
fn enclosing_error_check<'a>(node: Node<'a>, name: &str, source: &str) -> Option<Node<'a>> {
    let block = node.parent()?;
    let check = block.parent()?;
    (check.kind() == "if_statement" && is_nil_check(check, Some(name), source)).then_some(check)
}

/// `if X != nil`, where X is `name` or, when `name` is `None`, any error-ish
/// identifier (`err`, `parseErr`).
pub(crate) fn is_nil_check(if_statement: Node<'_>, name: Option<&str>, source: &str) -> bool {
    let Some(condition) = if_statement.child_by_field_name("condition") else {
        return false;
    };
    if condition.kind() != "binary_expression" {
        return false;
    }
    let (Some(left), Some(right)) = (
        condition.child_by_field_name("left"),
        condition.child_by_field_name("right"),
    ) else {
        return false;
    };
    let operator = condition
        .child_by_field_name("operator")
        .map(|operator| node_text(operator, source));
    let left_name = node_text(left, source);
    let named = match name {
        Some(name) => left_name == name,
        None => left_name == "err" || left_name.ends_with("Err"),
    };
    left.kind() == "identifier" && right.kind() == "nil" && operator == Some("!=") && named
}

/// The call whose result was assigned to `name` just before `check`: in the
/// `if`'s own initializer, or in the statement right before it.
fn origin_call<'a>(check: Node<'a>, name: &str, source: &str) -> Option<Node<'a>> {
    let assignment = check
        .child_by_field_name("initializer")
        .or_else(|| check.prev_named_sibling())?;
    if !matches!(
        assignment.kind(),
        "short_var_declaration" | "assignment_statement"
    ) {
        return None;
    }
    let left = assignment.child_by_field_name("left")?;
    let mut cursor = left.walk();
    let assigns = left
        .named_children(&mut cursor)
        .any(|target| node_text(target, source) == name);
    if !assigns {
        return None;
    }
    let right = assignment.child_by_field_name("right")?;
    let call = if right.kind() == "expression_list" {
        right.named_child(0)?
    } else {
        right
    };
    (call.kind() == "call_expression").then_some(call)
}

fn errors(file: &File, nodes: &[Node<'_>], source: &str) -> Count {
    let mut wrapped = 0u32;
    let mut bare = 0u32;
    for ret in nodes
        .iter()
        .filter(|node| node.kind() == "return_statement")
    {
        if last_returned(*ret).is_some_and(|value| is_wrapping(file, value, source)) {
            wrapped += 1;
        } else if is_bare_return(file, *ret, source) {
            bare += 1;
        }
    }
    Count::from_sides(wrapped, bare)
}

const STRUCTURED_LOGGERS: &[&str] = &[
    "log/slog",
    "go.uber.org/zap",
    "github.com/rs/zerolog",
    "github.com/rs/zerolog/log",
    "github.com/sirupsen/logrus",
];

const LEVELS: &[&str] = &["Debug", "Info", "Warn", "Warning", "Error"];

fn is_level(method: &str) -> bool {
    LEVELS.iter().any(|level| {
        method
            .strip_prefix(level)
            .is_some_and(|rest| matches!(rest, "" | "f" | "w" | "Context" | "ln"))
    })
}

/// A call through a structured logger: `slog.Info(…)`, `s.logger.Warn(…)`,
/// zerolog's `log.Info()`.
pub(crate) fn is_structured_log(file: &File, call: Node<'_>, source: &str) -> bool {
    let Some(function) = call
        .child_by_field_name("function")
        .filter(|function| function.kind() == "selector_expression")
    else {
        return false;
    };
    let (Some(operand), Some(field)) = (
        function.child_by_field_name("operand"),
        function.child_by_field_name("field"),
    ) else {
        return false;
    };
    if !is_level(node_text(field, source)) {
        return false;
    }
    // `app.Logger().Info(…)`, `zap.L().Warn(…)`: the logger comes from a call.
    if operand.kind() == "call_expression" {
        if package_call(file, operand, source)
            .is_some_and(|(path, _)| STRUCTURED_LOGGERS.contains(&path))
        {
            return true;
        }
        let Some(callee) = operand.child_by_field_name("function") else {
            return false;
        };
        return is_logger_name(node_text(callee, source));
    }
    let receiver = node_text(operand, source);
    if let Some(path) = file.package(receiver) {
        return STRUCTURED_LOGGERS.contains(&path);
    }
    is_logger_name(receiver)
}

/// `logger`, `s.log`, `app.Logger`: a value named as a logger.
fn is_logger_name(text: &str) -> bool {
    let last = text.rsplit('.').next().unwrap_or(text);
    matches!(
        last.to_ascii_lowercase().as_str(),
        "logger" | "log" | "slogger" | "zlog" | "lg"
    )
}

/// `fmt.Println(…)`, `fmt.Printf(…)` or the standard library's `log.Print*`.
pub(crate) fn is_print(file: &File, call: Node<'_>, source: &str) -> bool {
    matches!(
        package_call(file, call, source),
        Some(("fmt" | "log", "Print" | "Printf" | "Println"))
    )
}

/// Whether `node` is inside `func main`.
pub(crate) fn in_main(node: Node<'_>, source: &str) -> bool {
    let mut current = Some(node);
    while let Some(item) = current {
        if item.kind() == "function_declaration"
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

fn logging(file: &File, nodes: &[Node<'_>], source: &str) -> Count {
    let mut structured = 0u32;
    let mut printed = 0u32;
    for call in nodes.iter().filter(|node| node.kind() == "call_expression") {
        if in_main(*call, source) {
            continue;
        }
        if is_structured_log(file, *call, source) {
            structured += 1;
        } else if is_print(file, *call, source) {
            printed += 1;
        }
    }
    Count::from_sides(structured, printed)
}

/// Whether a function body has a guard other than an error check, and
/// whether it has an `if … else`. Function literals are their own bodies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Flow {
    pub(crate) has_guard: bool,
    pub(crate) has_else: bool,
}

pub(crate) fn flow_of(function: Node<'_>, source: &str) -> Flow {
    let mut flow = Flow::default();
    if let Some(body) = function.child_by_field_name("body") {
        visit_flow(body, source, &mut flow);
    }
    flow
}

fn visit_flow(node: Node<'_>, source: &str, flow: &mut Flow) {
    if node.kind() == "if_statement" {
        if node.child_by_field_name("alternative").is_some() {
            flow.has_else = true;
        } else if only_returns(node.child_by_field_name("consequence"))
            && !checks_an_error(node, source)
        {
            flow.has_guard = true;
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "func_literal" {
            continue;
        }
        // The `else if` of a chain is part of the outer `if`, not a second one.
        visit_flow(child, source, flow);
    }
}

/// Whether an `if` handles an error: its condition or initializer mentions
/// `err` or an `…Err` variable, as in `err != nil`, `errors.Is(err, X)` or
/// `if err := f(); err != nil && !retry`. All of them are the error idiom.
fn checks_an_error(if_statement: Node<'_>, source: &str) -> bool {
    ["condition", "initializer"].iter().any(|field| {
        if_statement
            .child_by_field_name(field)
            .is_some_and(|part| mentions_error(part, source))
    })
}

fn mentions_error(node: Node<'_>, source: &str) -> bool {
    if node.kind() == "identifier" {
        let name = node_text(node, source);
        return name == "err" || name.ends_with("Err");
    }
    let mut cursor = node.walk();
    let found = node
        .named_children(&mut cursor)
        .any(|child| mentions_error(child, source));
    found
}

fn only_returns(block: Option<Node<'_>>) -> bool {
    let Some(block) = block else {
        return false;
    };
    let mut cursor = block.walk();
    let statements: Vec<_> = block
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .collect();
    // tree-sitter-go wraps a block's statements in a `statement_list`.
    let statements = match statements.as_slice() {
        [list] if list.kind() == "statement_list" => {
            let mut inner = list.walk();
            list.named_children(&mut inner)
                .filter(|child| child.kind() != "comment")
                .collect()
        }
        _ => statements,
    };
    matches!(statements.as_slice(), [only] if only.kind() == "return_statement")
}

pub(crate) fn is_function(node: Node<'_>) -> bool {
    matches!(node.kind(), "function_declaration" | "method_declaration")
}

fn control_flow(nodes: &[Node<'_>], source: &str) -> Count {
    let mut guards = 0u32;
    let mut nested = 0u32;
    for function in nodes.iter().filter(|node| is_function(**node)) {
        let flow = flow_of(*function, source);
        guards += u32::from(flow.has_guard);
        nested += u32::from(flow.has_else);
    }
    Count::from_sides(guards, nested)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, SupportedLanguage};

    fn tally_at(path: &str, source: &str) -> GoTally {
        let tree = parse(SupportedLanguage::Go, source).unwrap();
        tally(Path::new(path), &tree, source)
    }

    fn tally_of(body: &str) -> GoTally {
        let source = format!(
            "package store\n\nimport (\n\t\"fmt\"\n\t\"log\"\n\t\"log/slog\"\n\t\"os\"\n\t\"encoding/json\"\n)\n\n{body}"
        );
        tally_at("internal/store/store.go", &source)
    }

    #[test]
    fn wrapped_errors_count_against_bare_returns_from_another_package() {
        let unanimous = tally_of(
            "func a(p string) error {\n\tif err := os.Remove(p); err != nil {\n\t\treturn fmt.Errorf(\"remove %s: %w\", p, err)\n\t}\n\treturn nil\n}\n\
             func b(d []byte, v any) error {\n\terr := json.Unmarshal(d, v)\n\tif err != nil {\n\t\treturn fmt.Errorf(\"decode: %w\", err)\n\t}\n\treturn nil\n}\n",
        );
        assert_eq!(unanimous.errors, Count::from_sides(2, 0));

        let mixed = tally_of(
            "func a(p string) error {\n\tif err := os.Remove(p); err != nil {\n\t\treturn err\n\t}\n\treturn nil\n}\n\
             func b(p string) (*os.File, error) {\n\tf, err := os.Open(p)\n\tif err != nil {\n\t\treturn nil, err\n\t}\n\treturn f, fmt.Errorf(\"x: %w\", e)\n}\n",
        );
        assert_eq!(mixed.errors, Count::from_sides(1, 2));

        let nothing = tally_of(
            "func a(s *Store) error {\n\tif err := s.flush(); err != nil {\n\t\treturn err\n\t}\n\tif err := save(); err != nil {\n\t\treturn err\n\t}\n\treturn fmt.Errorf(\"bad: %v\", e)\n}\n",
        );
        assert_eq!(
            nothing.errors,
            Count::default(),
            "own calls may already wrap, and %v does not wrap"
        );
    }

    #[test]
    fn structured_logging_counts_against_fmt_and_log_prints() {
        let unanimous = tally_of(
            "func a(s *Server) {\n\tslog.Info(\"start\", \"port\", 1)\n\ts.logger.Warn(\"slow\")\n\tlogger.Errorf(\"x %d\", 1)\n\ts.app.Logger().Error(\"failed\", \"err\", err)\n\terr.Error()\n}\n",
        );
        assert_eq!(unanimous.logging, Count::from_sides(4, 0));

        let mixed = tally_of(
            "func a() {\n\tslog.Debug(\"a\")\n\tfmt.Println(\"b\")\n\tlog.Printf(\"c %d\", 1)\n}\n",
        );
        assert_eq!(mixed.logging, Count::from_sides(1, 2));

        let nothing = tally_of(
            "func a(w io.Writer) {\n\tfmt.Fprintf(w, \"out\")\n\tfmt.Sprintf(\"x\")\n\tlog.Fatal(\"stop\")\n}\nfunc main() {\n\tfmt.Println(\"usage\")\n}\n",
        );
        assert_eq!(nothing.logging, Count::default());
    }

    #[test]
    fn commands_do_not_count_their_prints() {
        let source = "package main\n\nimport \"fmt\"\n\nfunc run() {\n\tfmt.Println(\"done\")\n}\n";
        assert_eq!(tally_at("tool.go", source).logging, Count::default());
        let library = source.replace("package main", "package tool");
        assert_eq!(
            tally_at("cmd/tool/run.go", &library).logging,
            Count::default()
        );
        assert_eq!(
            tally_at("tool/run.go", &library).logging,
            Count::from_sides(0, 1)
        );
    }

    #[test]
    fn early_returns_count_against_else_but_error_checks_do_not() {
        let unanimous = tally_of(
            "func a(x int) int {\n\tif x == 0 {\n\t\treturn 1\n\t}\n\treturn x\n}\n\
             func (s *S) b(u *User) bool {\n\tif u == nil {\n\t\treturn false\n\t}\n\treturn true\n}\n",
        );
        assert_eq!(unanimous.control_flow, Count::from_sides(2, 0));

        let mixed = tally_of(
            "func a(x int) int {\n\tif x == 0 {\n\t\treturn 1\n\t}\n\treturn x\n}\n\
             func b(x int) int {\n\tif x > 1 {\n\t\tx = 2\n\t} else if x > 0 {\n\t\tx = 1\n\t} else {\n\t\tx = 0\n\t}\n\treturn x\n}\n",
        );
        assert_eq!(mixed.control_flow, Count::from_sides(1, 1));

        let nothing = tally_of(
            "func a() error {\n\tif err := os.Remove(\"x\"); err != nil {\n\t\treturn err\n\t}\n\tif x {\n\t\tlog.Println(\"x\")\n\t}\n\treturn nil\n}\n\
             func b() error {\n\terr := run()\n\tif err != nil && !errors.Is(err, io.EOF) {\n\t\treturn err\n\t}\n\tif errors.Is(err, io.EOF) {\n\t\treturn nil\n\t}\n\tif parseErr != nil {\n\t\treturn parseErr\n\t}\n\treturn nil\n}\n",
        );
        assert_eq!(
            nothing.control_flow,
            Count::default(),
            "`if err != nil` is the idiom, not a guard-clause taste"
        );
    }
}
