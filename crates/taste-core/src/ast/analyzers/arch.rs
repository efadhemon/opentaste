//! Decorators and error classes.
//!
//! Where a name comes from beats what it is called. A decorator or exception
//! imported from a package belongs to a framework; one imported from a
//! relative or alias path belongs to the project. The name lists below are the
//! fallback for files that use a global or re-exported name, and they will
//! always have gaps, so the import check runs first.

use std::collections::HashSet;
use std::sync::OnceLock;

use tree_sitter::{Node, Tree};

use super::{run, Count};
use crate::ast::query::{self, ScriptQuery};
use crate::ast::SupportedLanguage;

static DECORATORS: ScriptQuery = ScriptQuery::new("(decorator) @decorator");

static IMPORTS: ScriptQuery = ScriptQuery::new("(import_statement source: (string) @source) @stmt");

static ERRORS: ScriptQuery = ScriptQuery::new(
    r#"
    (class_declaration name: (type_identifier) @name) @class
    (throw_statement
      (new_expression constructor: (identifier) @constructor)) @throw
    "#,
);

pub(super) fn decorators(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let origins = Origins::of(lang, tree, source);
    let mut custom = 0u32;
    let mut framework = 0u32;
    for (_, node) in run(&DECORATORS, lang, tree, source) {
        let Some(name) = decorator_name(node, source).filter(|name| !name.is_empty()) else {
            continue;
        };
        if origins.is_framework(&name, is_framework_decorator) {
            framework += 1;
        } else {
            custom += 1;
        }
    }
    Count::from_sides(custom, framework)
}

pub(super) fn errors(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    let origins = Origins::of(lang, tree, source);
    let mut custom = 0u32;
    let mut bare = 0u32;
    for (index, node) in run(&ERRORS, lang, tree, source) {
        match capture(index, lang) {
            "class" => {
                if extends_error_base(node, source) {
                    custom += 1;
                }
            }
            "constructor" => {
                let name = query::node_text(node, source);
                if origins.packages.contains(name) {
                    continue;
                }
                match classify_throw(name) {
                    ThrowKind::Custom => custom += 1,
                    ThrowKind::Bare => bare += 1,
                    ThrowKind::Ignore => {}
                }
            }
            _ => {}
        }
    }
    Count::from_sides(custom, bare)
}

/// Names a file imports, split by whether the specifier is a package.
pub(crate) struct Origins {
    pub(crate) packages: HashSet<String>,
    pub(crate) project: HashSet<String>,
}

impl Origins {
    pub(crate) fn of(lang: SupportedLanguage, tree: &Tree, source: &str) -> Self {
        let mut origins = Origins {
            packages: HashSet::new(),
            project: HashSet::new(),
        };
        let Some(compiled) = IMPORTS.get(lang) else {
            return origins;
        };
        let names = compiled.capture_names();
        for group in query::matches(compiled, tree, source) {
            let node = |wanted: &str| {
                group
                    .iter()
                    .find(|(index, _)| names.get(*index) == Some(&wanted))
                    .map(|(_, node)| *node)
            };
            let (Some(stmt), Some(specifier)) = (node("stmt"), node("source")) else {
                continue;
            };
            let specifier = query::unquote(query::node_text(specifier, source));
            let target = if query::is_package_specifier(specifier) {
                &mut origins.packages
            } else {
                &mut origins.project
            };
            target.extend(query::import_locals(stmt, source));
        }
        origins
    }

    /// Framework when imported from a package, project-specific when imported
    /// from the project, and otherwise whatever `known` says about the name.
    pub(crate) fn is_framework(&self, name: &str, known: fn(&str) -> bool) -> bool {
        if self.packages.contains(name) {
            return true;
        }
        if self.project.contains(name) {
            return false;
        }
        known(name)
    }
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

/// A thrown class the project defined itself rather than one a runtime or
/// framework ships. Shared with the structural diff so both agree.
pub(crate) fn is_custom_error(name: &str) -> bool {
    matches!(classify_throw(name), ThrowKind::Custom)
}

/// Errors that ship with JavaScript, the DOM, or NestJS. They end in `Error`
/// or `Exception` but say nothing about a project's own error classes.
fn is_builtin_error(name: &str) -> bool {
    matches!(
        name,
        "TypeError"
            | "RangeError"
            | "SyntaxError"
            | "ReferenceError"
            | "EvalError"
            | "URIError"
            | "AggregateError"
            | "DOMException"
            | "BadGatewayException"
            | "BadRequestException"
            | "ConflictException"
            | "ForbiddenException"
            | "GatewayTimeoutException"
            | "GoneException"
            | "HttpVersionNotSupportedException"
            | "ImATeapotException"
            | "InternalServerErrorException"
            | "MethodNotAllowedException"
            | "MisdirectedException"
            | "NotAcceptableException"
            | "NotFoundException"
            | "NotImplementedException"
            | "PayloadTooLargeException"
            | "PreconditionFailedException"
            | "RequestTimeoutException"
            | "ServiceUnavailableException"
            | "UnauthorizedException"
            | "UnprocessableEntityException"
            | "UnsupportedMediaTypeException"
            | "RpcException"
            | "WsException"
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

/// A decorator that ships with a framework the scanner knows by name.
pub(crate) fn is_framework_decorator(name: &str) -> bool {
    framework_names().contains(name)
}

fn framework_names() -> &'static HashSet<&'static str> {
    static NAMES: OnceLock<HashSet<&'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        [
            "AfterInsert",
            "AfterLoad",
            "AfterRemove",
            "AfterUpdate",
            "All",
            "Allow",
            "ApiBearerAuth",
            "ApiBody",
            "ApiOperation",
            "ApiParam",
            "ApiProperty",
            "ApiPropertyOptional",
            "ApiQuery",
            "ApiResponse",
            "ApiTags",
            "Args",
            "ArgsType",
            "ArrayMaxSize",
            "ArrayMinSize",
            "ArrayNotEmpty",
            "BeforeInsert",
            "BeforeRemove",
            "BeforeUpdate",
            "Body",
            "Catch",
            "Check",
            "Column",
            "Component",
            "ConnectedSocket",
            "Contains",
            "ContentChild",
            "ContentChildren",
            "Context",
            "Controller",
            "CreateDateColumn",
            "Cron",
            "Ctx",
            "Delete",
            "DeleteDateColumn",
            "Directive",
            "Entity",
            "Equals",
            "EventPattern",
            "Exclude",
            "Expose",
            "Field",
            "Get",
            "Global",
            "Head",
            "Header",
            "Headers",
            "HostBinding",
            "HostListener",
            "HostParam",
            "HttpCode",
            "Index",
            "Inject",
            "InjectConnection",
            "InjectDataSource",
            "InjectModel",
            "InjectRepository",
            "Injectable",
            "Input",
            "InputType",
            "Interval",
            "Ip",
            "IsAlpha",
            "IsAlphanumeric",
            "IsArray",
            "IsBoolean",
            "IsDate",
            "IsDateString",
            "IsDefined",
            "IsEmail",
            "IsEmpty",
            "IsEnum",
            "IsISO8601",
            "IsIn",
            "IsInt",
            "IsJSON",
            "IsNotEmpty",
            "IsNotEmptyObject",
            "IsNumber",
            "IsNumberString",
            "IsObject",
            "IsOptional",
            "IsPhoneNumber",
            "IsPositive",
            "IsString",
            "IsUUID",
            "IsUrl",
            "JoinColumn",
            "JoinTable",
            "Length",
            "ManyToMany",
            "ManyToOne",
            "Matches",
            "Max",
            "MaxLength",
            "MessageBody",
            "MessagePattern",
            "Min",
            "MinLength",
            "Module",
            "Mutation",
            "Next",
            "NgModule",
            "NotEquals",
            "ObjectType",
            "OnEvent",
            "OneToMany",
            "OneToOne",
            "Optional",
            "Options",
            "Output",
            "Param",
            "Patch",
            "Payload",
            "Pipe",
            "Post",
            "PrimaryColumn",
            "PrimaryGeneratedColumn",
            "Prop",
            "Put",
            "Query",
            "Redirect",
            "RelationId",
            "Render",
            "Req",
            "Res",
            "ResolveField",
            "Resolver",
            "Schema",
            "Session",
            "SetMetadata",
            "Sse",
            "SubscribeMessage",
            "Subscription",
            "Timeout",
            "Transform",
            "Tree",
            "TreeChildren",
            "TreeParent",
            "Type",
            "Unique",
            "UpdateDateColumn",
            "UploadedFile",
            "UploadedFiles",
            "UseFilters",
            "UseGuards",
            "UseInterceptors",
            "UsePipes",
            "ValidateIf",
            "ValidateNested",
            "Version",
            "VersionColumn",
            "ViewChild",
            "ViewChildren",
            "ViewColumn",
            "ViewEntity",
            "WebSocketGateway",
            "WebSocketServer",
            "action",
            "computed",
            "customElement",
            "observable",
            "property",
            "query",
            "state",
        ]
        .into_iter()
        .collect()
    })
}
