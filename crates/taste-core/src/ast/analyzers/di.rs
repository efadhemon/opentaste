//! Dependency injection in NestJS and Angular classes.
//!
//! DI looks too different across frameworks to count in general, so only
//! the two frameworks that make the choice visible are counted, and only in
//! classes they manage: a class decorated with one of their DI decorators
//! *imported from the framework*. A look-alike `@Injectable` from anywhere
//! else counts nothing.
//!
//! Within those classes:
//!
//! - NestJS: constructor parameters against `@Inject()` on a property.
//! - Angular: constructor parameters against the `inject()` function.
//! - Both: injected dependencies against a project service built with `new`.
//!   Only classes the project imports from its own files (or defines in the
//!   same file) and names as a service count: `new Date()`, `new Map()`,
//!   `new Logger()` and a package's `new S3Client()` are not services.

use std::collections::HashMap;

use tree_sitter::{Node, Tree};

use super::Count;
use crate::ast::query::{import_locals, is_package_specifier, node_text, unquote};
use crate::ast::SupportedLanguage;

/// The frameworks whose dependency injection is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Framework {
    Nest,
    Angular,
}

/// Per-file DI counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Injection {
    /// NestJS: `left` constructor parameters, `right` `@Inject()` properties.
    pub nest: Count,
    /// Angular: `left` constructor parameters, `right` `inject()` calls.
    pub angular: Count,
    /// `left` injected dependencies of either kind, `right` project services
    /// built with `new` inside a managed class.
    pub constructed: Count,
}

/// Local name → import specifier, for every name a file imports.
pub(crate) fn import_sources(tree: &Tree, source: &str) -> HashMap<String, String> {
    let mut sources = HashMap::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Some(specifier) = statement.child_by_field_name("source") else {
            continue;
        };
        let specifier = unquote(node_text(specifier, source)).to_string();
        for local in import_locals(statement, source) {
            sources.insert(local, specifier.clone());
        }
    }
    sources
}

/// The framework managing `class`, from its decorators and where they come from.
pub(crate) fn framework_of(
    class: Node<'_>,
    source: &str,
    imports: &HashMap<String, String>,
) -> Option<Framework> {
    decorators_of(class).into_iter().find_map(|decorator| {
        let name = super::arch::decorator_name(decorator, source)?;
        let from = imports.get(&name)?;
        match (from.as_str(), name.as_str()) {
            (
                "@nestjs/common" | "@nestjs/graphql" | "@nestjs/websockets",
                "Injectable" | "Controller" | "Resolver" | "WebSocketGateway",
            ) => Some(Framework::Nest),
            ("@angular/core", "Injectable" | "Component" | "Directive" | "Pipe") => {
                Some(Framework::Angular)
            }
            _ => None,
        }
    })
}

/// A class's decorators: its own children, or the `export` statement's when
/// the class is exported, which is where the grammar attaches them.
fn decorators_of(class: Node<'_>) -> Vec<Node<'_>> {
    let mut found = Vec::new();
    for holder in [
        Some(class),
        class.parent().filter(|p| p.kind() == "export_statement"),
    ]
    .into_iter()
    .flatten()
    {
        let mut cursor = holder.walk();
        found.extend(
            holder
                .children(&mut cursor)
                .filter(|child| child.kind() == "decorator"),
        );
    }
    found
}

/// Every class in the file with the framework managing it.
pub(crate) fn managed_classes<'a>(
    tree: &'a Tree,
    source: &str,
    imports: &HashMap<String, String>,
) -> Vec<(Node<'a>, Framework)> {
    module_classes(tree)
        .into_iter()
        .filter_map(|class| Some((class, framework_of(class, source, imports)?)))
        .collect()
}

/// The classes a module declares at its top level, exported or not. Nest and
/// Angular register only these, so the rest of the tree is not walked.
fn module_classes(tree: &Tree) -> Vec<Node<'_>> {
    let root = tree.root_node();
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .filter_map(|statement| match statement.kind() {
            "class_declaration" => Some(statement),
            "export_statement" => statement
                .child_by_field_name("declaration")
                .filter(|declaration| declaration.kind() == "class_declaration"),
            _ => None,
        })
        .collect()
}

/// How a managed class receives its dependencies.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Dependencies {
    /// Types of constructor parameters, or `""` when untyped.
    pub(crate) constructor: Vec<String>,
    /// Arguments of `inject(X)` field initialisers.
    pub(crate) injected: Vec<String>,
    /// Fields carrying `@Inject(…)` and no initialiser.
    pub(crate) properties: usize,
    /// `(class, text)` of each project service built with `new`.
    pub(crate) constructed: Vec<(String, String)>,
}

pub(crate) fn dependencies(
    class: Node<'_>,
    source: &str,
    imports: &HashMap<String, String>,
    local_classes: &[String],
) -> Dependencies {
    let mut deps = Dependencies::default();
    let Some(body) = class.child_by_field_name("body") else {
        return deps;
    };
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        match member.kind() {
            "method_definition"
                if member
                    .child_by_field_name("name")
                    .is_some_and(|name| node_text(name, source) == "constructor") =>
            {
                if let Some(params) = member.child_by_field_name("parameters") {
                    let mut inner = params.walk();
                    for param in params.named_children(&mut inner) {
                        if matches!(param.kind(), "required_parameter" | "optional_parameter") {
                            let ty = param
                                .child_by_field_name("type")
                                .map(|ty| node_text(ty, source).trim_start_matches(':').trim())
                                .unwrap_or("");
                            deps.constructor.push(ty.to_string());
                        }
                    }
                }
            }
            "public_field_definition" => {
                let value = member.child_by_field_name("value");
                if let Some(argument) = value.and_then(|value| inject_call(value, source, imports))
                {
                    deps.injected.push(argument);
                } else if value.is_none() && has_inject_decorator(member, source, imports) {
                    deps.properties += 1;
                }
            }
            _ => {}
        }
    }
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        if node.kind() == "new_expression" {
            if let Some(class) = node.child_by_field_name("constructor") {
                let name = node_text(class, source);
                if is_project_service(name, imports, local_classes) {
                    deps.constructed
                        .push((name.to_string(), node_text(node, source).to_string()));
                }
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    deps
}

/// Where `class` now receives `service`: the constructor parameter typed
/// with it, or the field initialised with `inject(service)`.
pub(crate) fn injection_site(
    class: Node<'_>,
    source: &str,
    imports: &HashMap<String, String>,
    service: &str,
) -> Option<String> {
    let body = class.child_by_field_name("body")?;
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        match member.kind() {
            "method_definition"
                if member
                    .child_by_field_name("name")
                    .is_some_and(|name| node_text(name, source) == "constructor") =>
            {
                let params = member.child_by_field_name("parameters")?;
                let mut inner = params.walk();
                let found = params.named_children(&mut inner).find(|param| {
                    param.child_by_field_name("type").is_some_and(|ty| {
                        node_text(ty, source).trim_start_matches(':').trim() == service
                    })
                });
                if let Some(param) = found {
                    return Some(node_text(param, source).to_string());
                }
            }
            "public_field_definition" => {
                let value = member.child_by_field_name("value");
                if value
                    .and_then(|value| inject_call(value, source, imports))
                    .as_deref()
                    == Some(service)
                {
                    return Some(node_text(member, source).to_string());
                }
            }
            _ => {}
        }
    }
    None
}

/// `inject(X)` from `@angular/core`, returning `X`.
fn inject_call(value: Node<'_>, source: &str, imports: &HashMap<String, String>) -> Option<String> {
    if value.kind() != "call_expression" {
        return None;
    }
    let function = value.child_by_field_name("function")?;
    let name = node_text(function, source);
    if imports.get(name).map(String::as_str) != Some("@angular/core") || name != "inject" {
        return None;
    }
    let argument = value.child_by_field_name("arguments")?.named_child(0)?;
    Some(node_text(argument, source).to_string())
}

fn has_inject_decorator(field: Node<'_>, source: &str, imports: &HashMap<String, String>) -> bool {
    let mut cursor = field.walk();
    let found = field.children(&mut cursor).any(|child| {
        child.kind() == "decorator"
            && super::arch::decorator_name(child, source).is_some_and(|name| {
                name == "Inject"
                    && imports
                        .get(&name)
                        .is_some_and(|from| from.starts_with("@nestjs/"))
            })
    });
    found
}

const SERVICE_SUFFIXES: &[&str] = &["Service", "Repository"];

/// A class the project wrote and named as a service.
pub(crate) fn is_project_service(
    name: &str,
    imports: &HashMap<String, String>,
    local_classes: &[String],
) -> bool {
    let named = SERVICE_SUFFIXES
        .iter()
        .any(|suffix| name.len() > suffix.len() && name.ends_with(suffix));
    let from_project = match imports.get(name) {
        Some(from) => !is_package_specifier(from),
        None => local_classes.iter().any(|local| local == name),
    };
    named && from_project
}

/// Names of the classes the file declares at its top level.
pub(crate) fn local_classes(tree: &Tree, source: &str) -> Vec<String> {
    module_classes(tree)
        .into_iter()
        .filter_map(|class| class.child_by_field_name("name"))
        .map(|name| node_text(name, source).to_string())
        .collect()
}

pub(super) fn injection(lang: SupportedLanguage, tree: &Tree, source: &str) -> Injection {
    let mut counts = Injection::default();
    if !lang.is_typescript() {
        return counts;
    }
    let imports = import_sources(tree, source);
    // Only a file importing from a DI framework can have a managed class.
    let from_framework = imports
        .values()
        .any(|from| from.starts_with("@nestjs/") || from == "@angular/core");
    if !from_framework {
        return counts;
    }
    let locals = local_classes(tree, source);
    let (mut nest, mut angular, mut constructed) = ((0, 0), (0, 0), (0, 0));
    for (class, framework) in managed_classes(tree, source, &imports) {
        let deps = dependencies(class, source, &imports, &locals);
        let constructor = deps.constructor.len() as u32;
        match framework {
            Framework::Nest => {
                nest.0 += constructor;
                nest.1 += deps.properties as u32;
            }
            Framework::Angular => {
                angular.0 += constructor;
                angular.1 += deps.injected.len() as u32;
            }
        }
        constructed.0 += constructor + deps.injected.len() as u32 + deps.properties as u32;
        constructed.1 += deps.constructed.len() as u32;
    }
    counts.nest = Count::from_sides(nest.0, nest.1);
    counts.angular = Count::from_sides(angular.0, angular.1);
    counts.constructed = Count::from_sides(constructed.0, constructed.1);
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;

    fn of(source: &str) -> Injection {
        let tree = parse(SupportedLanguage::TypeScript, source).unwrap();
        injection(SupportedLanguage::TypeScript, &tree, source)
    }

    const NEST: &str = "import { Injectable, Controller, Inject } from '@nestjs/common';\n";
    const ANGULAR: &str = "import { Component, Injectable, inject } from '@angular/core';\n";

    #[test]
    fn nest_constructor_injection_counts_against_property_injection() {
        let constructor = of(&format!(
            "{NEST}import {{ UsersService }} from './users.service';\n@Injectable()\nexport class A {{\n  constructor(private readonly users: UsersService, private readonly mail: MailService) {{}}\n}}\n"
        ));
        assert_eq!(constructor.nest, Count::from_sides(2, 0));
        assert_eq!(constructor.constructed, Count::from_sides(2, 0));

        let mixed = of(&format!(
            "{NEST}@Controller('x')\nclass B {{\n  @Inject(CACHE) private cache: Cache;\n  constructor(private users: UsersService) {{}}\n}}\n"
        ));
        assert_eq!(mixed.nest, Count::from_sides(1, 1));
    }

    #[test]
    fn angular_constructor_injection_counts_against_the_inject_function() {
        let modern = of(&format!(
            "{ANGULAR}@Component({{ selector: 'x' }})\nexport class A {{\n  private readonly http = inject(HttpClient);\n  private readonly users = inject(UsersService);\n}}\n"
        ));
        assert_eq!(modern.angular, Count::from_sides(0, 2));

        let classic = of(&format!(
            "{ANGULAR}@Injectable({{ providedIn: 'root' }})\nexport class B {{\n  constructor(private http: HttpClient) {{}}\n}}\n"
        ));
        assert_eq!(classic.angular, Count::from_sides(1, 0));
    }

    #[test]
    fn a_project_service_built_with_new_counts_against_injection() {
        let built = of(&format!(
            "{NEST}import {{ UsersRepository }} from '../users/users.repository';\nimport {{ S3Client }} from '@aws-sdk/client-s3';\n@Injectable()\nexport class A {{\n  private readonly repo = new UsersRepository();\n  private readonly s3 = new S3Client({{}});\n  private readonly seen = new Map();\n  private readonly logger = new Logger(A.name);\n  at() {{ return new Date(); }}\n}}\n"
        ));
        assert_eq!(
            built.constructed,
            Count::from_sides(0, 1),
            "only the project's own service counts; Date, Map, Logger and a package client do not"
        );
    }

    #[test]
    fn a_look_alike_decorator_or_an_undecorated_class_counts_nothing() {
        for source in [
            "import { Injectable } from './my-di';\n@Injectable()\nexport class A {\n  constructor(private users: UsersService) {}\n}\n".to_string(),
            "export class Plain {\n  constructor(private users: UsersService) {}\n  private repo = new UsersRepository();\n}\n".to_string(),
            format!("{NEST}export class Undecorated {{\n  constructor(private users: UsersService) {{}}\n}}\n"),
        ] {
            assert_eq!(of(&source), Injection::default(), "{source}");
        }
    }
}
