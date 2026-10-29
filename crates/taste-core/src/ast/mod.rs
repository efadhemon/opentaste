//! Cold-start repository scanner.
//!
//! Analyzers count. [`promote`] is the only place that turns a count into a
//! rule, and [`scan`] is the only place that writes one.

mod analyzers;
mod parser;
mod promote;
mod query;
mod scan;

pub(crate) use analyzers::decorator_name;
pub use analyzers::{analyze_source, Count, Tally};
pub use parser::{content_hash, parse, SupportedLanguage};
pub use promote::{
    observations, opposite_instruction, MIN_FILES, MIN_RATIO, PREFER_ARROW_FUNCTIONS,
    PREFER_CUSTOM_ERRORS, PREFER_FUNCTION_DECLARATIONS, PREFER_GUARD_CLAUSES, PREFER_INTERFACE,
    PREFER_PATH_ALIASES, PREFER_TYPE_ALIAS,
};
pub(crate) use query::{matches, node_text, unquote, ScriptQuery};
pub use scan::{scan, ScanReport};
