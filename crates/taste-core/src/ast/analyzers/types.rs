//! `interface` declarations versus `type` aliases for object types.
//!
//! JavaScript files are skipped entirely. An `interface` cannot appear in
//! JavaScript, so counting a file that could not have expressed either side
//! would dilute the ratio with zeros.
//!
//! Only an alias whose value is an object type counts. Unions, function
//! types, mapped types and utility types cannot be written as an `interface`,
//! so counting them would push the ratio toward "prefer `type`" in projects
//! that always use `interface` for objects.

use tree_sitter::Tree;

use super::{run, Count};
use crate::ast::query::ScriptQuery;
use crate::ast::SupportedLanguage;

static TYPES: ScriptQuery = ScriptQuery::new(
    r#"
    (interface_declaration) @interface
    (type_alias_declaration value: (object_type)) @type_alias
    "#,
);

pub(super) fn type_constructs(lang: SupportedLanguage, tree: &Tree, source: &str) -> Count {
    if !lang.is_typescript() {
        return Count::default();
    }
    let mut interfaces = 0u32;
    let mut aliases = 0u32;
    for (index, _) in run(&TYPES, lang, tree, source) {
        match capture(&TYPES, lang, index) {
            "interface" => interfaces += 1,
            "type_alias" => aliases += 1,
            _ => {}
        }
    }
    Count::from_sides(interfaces, aliases)
}

fn capture(query: &ScriptQuery, lang: SupportedLanguage, index: usize) -> &str {
    query
        .get(lang)
        .and_then(|compiled| compiled.capture_names().get(index).copied())
        .unwrap_or("")
}
