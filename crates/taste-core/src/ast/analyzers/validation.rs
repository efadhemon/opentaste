//! Which validation library a file uses, and how zod types are written.
//!
//! A file counts for a library only when it imports it as a value *and*
//! uses that import outside a type position: `import type { ZodType }`, or a
//! zod import used only in `z.infer<…>`, validates nothing. Files that
//! validate nothing count for no library, so one schema in a utility file
//! cannot pass for a convention: the promotion floor is ten validating files.

use std::collections::HashSet;

use tree_sitter::{Node, Tree};

use super::Count;
use crate::ast::query::{node_text, unquote};
use crate::ast::SupportedLanguage;

/// A schema-validation library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Library {
    Zod,
    Yup,
    Joi,
    ClassValidator,
    Valibot,
    IoTs,
}

impl Library {
    pub const ALL: [Library; 6] = [
        Library::Zod,
        Library::Yup,
        Library::Joi,
        Library::ClassValidator,
        Library::Valibot,
        Library::IoTs,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Library::Zod => "zod",
            Library::Yup => "yup",
            Library::Joi => "joi",
            Library::ClassValidator => "class-validator",
            Library::Valibot => "valibot",
            Library::IoTs => "io-ts",
        }
    }

    /// The library an import specifier loads, if it is one of these.
    pub fn of_module(specifier: &str) -> Option<Library> {
        let package = |name: &str| specifier == name || specifier.starts_with(&format!("{name}/"));
        Library::ALL.into_iter().find(|library| match library {
            Library::Joi => package("joi") || package("@hapi/joi"),
            other => package(other.name()),
        })
    }

    fn index(self) -> usize {
        Library::ALL
            .iter()
            .position(|library| *library == self)
            .unwrap_or(0)
    }
}

/// Per-file validation counts. `libraries[i].files` is 1 when the file uses
/// [`Library::ALL`]`[i]`; `left` is how many value uses it has.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Validation {
    pub libraries: [Count; 6],
    /// `files` is 1 when the file uses any library.
    pub validating: Count,
    /// `left` is a type derived with `z.infer<typeof S>` (or `z.input`,
    /// `z.output`). `right` is a hand-written interface or object type named
    /// after a schema in the same file (`UserSchema` and `interface User`).
    pub schema_types: Count,
}

/// The local names a file imports from each library as values.
pub(crate) fn imported(tree: &Tree, source: &str) -> Vec<(Library, String)> {
    let mut found = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.kind() != "import_statement" {
            continue;
        }
        let Some(library) = statement
            .child_by_field_name("source")
            .and_then(|specifier| Library::of_module(unquote(node_text(specifier, source))))
        else {
            continue;
        };
        if node_text(statement, source).starts_with("import type") {
            continue;
        }
        collect_value_locals(statement, source, &mut |name| found.push((library, name)));
    }
    found
}

fn collect_value_locals(node: Node<'_>, source: &str, push: &mut dyn FnMut(String)) {
    match node.kind() {
        "import_specifier" => {
            if node_text(node, source).starts_with("type ") {
                return;
            }
            let local = node
                .child_by_field_name("alias")
                .or_else(|| node.child_by_field_name("name"));
            if let Some(local) = local {
                push(node_text(local, source).to_string());
            }
        }
        "identifier" => push(node_text(node, source).to_string()),
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() != "string" {
                    collect_value_locals(child, source, push);
                }
            }
        }
    }
}

/// Node kinds below which a name is used as a type, not a value.
fn is_type_position(kind: &str) -> bool {
    matches!(
        kind,
        "import_statement"
            | "type_annotation"
            | "type_alias_declaration"
            | "type_arguments"
            | "type_parameters"
            | "interface_declaration"
            | "implements_clause"
            | "generic_type"
            | "nested_type_identifier"
            | "type_query"
    )
}

/// How many times each of `names` is used as a value.
pub(crate) fn value_uses(tree: &Tree, source: &str, names: &HashSet<&str>) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if is_type_position(node.kind()) {
            continue;
        }
        if node.kind() == "identifier" && names.contains(node_text(node, source)) {
            found.push(node_text(node, source).to_string());
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    found
}

/// Which libraries `tree` uses as values, each with its local names.
pub(crate) fn used_libraries(tree: &Tree, source: &str) -> Vec<(Library, HashSet<String>)> {
    let imports = imported(tree, source);
    let names: HashSet<&str> = imports.iter().map(|(_, name)| name.as_str()).collect();
    let uses = value_uses(tree, source, &names);
    let mut found: Vec<(Library, HashSet<String>)> = Vec::new();
    for (library, name) in &imports {
        if !uses.contains(name) {
            continue;
        }
        match found.iter_mut().find(|(seen, _)| seen == library) {
            Some((_, locals)) => {
                locals.insert(name.clone());
            }
            None => found.push((*library, HashSet::from([name.clone()]))),
        }
    }
    found
}

pub(super) fn validation(lang: SupportedLanguage, tree: &Tree, source: &str) -> Validation {
    let mut counts = Validation::default();
    let imports = imported(tree, source);
    let names: HashSet<&str> = imports.iter().map(|(_, name)| name.as_str()).collect();
    let uses = value_uses(tree, source, &names);
    for (library, name) in &imports {
        let used = uses.iter().filter(|used| *used == name).count() as u32;
        if used == 0 {
            continue;
        }
        let slot = &mut counts.libraries[library.index()];
        slot.left += used;
        slot.files = 1;
        counts.validating.files = 1;
    }
    if lang.is_typescript() {
        let zod: Vec<&str> = imports
            .iter()
            .filter(|(library, _)| *library == Library::Zod)
            .map(|(_, name)| name.as_str())
            .collect();
        // A file that only derives types from a schema defined elsewhere
        // still chose `z.infer`, so zod need not be used as a value here.
        if !zod.is_empty() {
            counts.schema_types = schema_types(tree, source, &zod);
        }
    }
    counts
}

/// `z.infer<typeof S>` aliases against hand-written twins of a schema.
fn schema_types(tree: &Tree, source: &str, zod: &[&str]) -> Count {
    let root = tree.root_node();
    let mut schemas = HashSet::new();
    let mut declared = Vec::new();
    let mut inferred = 0u32;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "variable_declarator" => {
                let rooted = node
                    .child_by_field_name("value")
                    .is_some_and(|value| rooted_at(value, source, zod));
                if let (true, Some(name)) = (rooted, node.child_by_field_name("name")) {
                    schemas.insert(node_text(name, source).to_string());
                }
            }
            "type_alias_declaration" => {
                let value = node.child_by_field_name("value");
                if value.is_some_and(|value| is_inferred(value, source, zod)) {
                    inferred += 1;
                } else if value.is_some_and(|value| value.kind() == "object_type") {
                    declared.extend(
                        node.child_by_field_name("name")
                            .map(|n| node_text(n, source)),
                    );
                }
            }
            "interface_declaration" => {
                declared.extend(
                    node.child_by_field_name("name")
                        .map(|n| node_text(n, source)),
                );
            }
            _ => {}
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    let twins = declared
        .iter()
        .filter(|name| {
            schemas
                .iter()
                .any(|schema| twin_of(schema).as_deref() == Some(**name))
        })
        .count() as u32;
    Count::from_sides(inferred, twins)
}

/// `User` for `UserSchema` or `userSchema`.
fn twin_of(schema: &str) -> Option<String> {
    let stem = schema.strip_suffix("Schema")?;
    let mut chars = stem.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

/// Whether an expression is a call chain starting at one of `names`:
/// `z.object({…}).strict()`.
fn rooted_at(mut node: Node<'_>, source: &str, names: &[&str]) -> bool {
    loop {
        node = match node.kind() {
            "call_expression" => match node.child_by_field_name("function") {
                Some(function) => function,
                None => return false,
            },
            "member_expression" => match node.child_by_field_name("object") {
                Some(object) => object,
                None => return false,
            },
            "identifier" => return names.contains(&node_text(node, source)),
            _ => return false,
        };
    }
}

fn is_inferred(value: Node<'_>, source: &str, zod: &[&str]) -> bool {
    if value.kind() != "generic_type" {
        return false;
    }
    let Some(name) = value.child_by_field_name("name") else {
        return false;
    };
    if name.kind() != "nested_type_identifier" {
        return false;
    }
    let module = name
        .child_by_field_name("module")
        .map(|m| node_text(m, source));
    let member = name
        .child_by_field_name("name")
        .map(|m| node_text(m, source));
    module.is_some_and(|module| zod.contains(&module))
        && matches!(member, Some("infer" | "input" | "output"))
}

/// Whether `value` is a schema built with `library`'s locals: used by the
/// structural diff to pair a yup schema with the zod one that replaced it.
pub(crate) fn schema_library(
    value: Node<'_>,
    source: &str,
    libraries: &[(Library, HashSet<String>)],
) -> Option<Library> {
    libraries.iter().find_map(|(library, locals)| {
        let names: Vec<&str> = locals.iter().map(String::as_str).collect();
        rooted_at(value, source, &names).then_some(*library)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::parse;

    fn of(source: &str) -> Validation {
        let tree = parse(SupportedLanguage::TypeScript, source).unwrap();
        validation(SupportedLanguage::TypeScript, &tree, source)
    }

    fn files(counts: &Validation) -> Vec<(&'static str, u32)> {
        Library::ALL
            .iter()
            .zip(counts.libraries)
            .filter(|(_, count)| count.files > 0)
            .map(|(library, count)| (library.name(), count.left))
            .collect()
    }

    #[test]
    fn a_file_counts_for_the_library_it_imports_and_uses() {
        let zod =
            of("import { z } from \"zod\";\nexport const User = z.object({ name: z.string() });\n");
        assert_eq!(files(&zod), vec![("zod", 2)]);
        assert_eq!(zod.validating.files, 1);

        let yup = of("import * as yup from 'yup';\nconst s = yup.object({ a: yup.string() });\n");
        assert_eq!(files(&yup), vec![("yup", 2)]);

        let decorators = of(
            "import { IsString, Length } from 'class-validator';\nclass Dto {\n  @IsString() @Length(1, 9) name: string;\n}\n",
        );
        assert_eq!(files(&decorators), vec![("class-validator", 2)]);

        let joi = of("import Joi from '@hapi/joi';\nconst s = Joi.object({});\n");
        assert_eq!(files(&joi), vec![("joi", 1)]);
    }

    #[test]
    fn type_only_imports_and_type_positions_validate_nothing() {
        for source in [
            "import type { ZodType } from 'zod';\nexport function f(s: ZodType) {}\n",
            "import { type ZodType } from 'zod';\nlet s: ZodType;\n",
            "import { z } from 'zod';\nimport { UserSchema } from './schema';\nexport type User = z.infer<typeof UserSchema>;\n",
            "import { z } from 'zod';\n",
            "export const nothing = 1;\n",
        ] {
            let counts = of(source);
            assert_eq!(counts.validating.files, 0, "{source}");
            assert!(files(&counts).is_empty(), "{source}");
        }
    }

    #[test]
    fn inferred_types_count_against_hand_written_twins() {
        let inferred = of(
            "import { z } from 'zod';\nexport const UserSchema = z.object({ name: z.string() }).strict();\nexport type User = z.infer<typeof UserSchema>;\nexport type UserInput = z.input<typeof UserSchema>;\n",
        );
        assert_eq!(inferred.schema_types, Count::from_sides(2, 0));

        let twin = of(
            "import { z } from 'zod';\nexport const userSchema = z.object({ name: z.string() });\nexport interface User { name: string }\ninterface Unrelated { id: number }\n",
        );
        assert_eq!(
            twin.schema_types,
            Count::from_sides(0, 1),
            "only the type named after a schema is its twin"
        );

        let elsewhere = of(
            "import { z } from 'zod';\nimport { S } from './s';\nexport type User = z.infer<typeof S>;\n",
        );
        assert_eq!(
            elsewhere.schema_types,
            Count::from_sides(1, 0),
            "deriving from a schema in another file is still the derived style"
        );
        assert_eq!(
            elsewhere.validating.files, 0,
            "but it validates nothing here"
        );
    }
}
