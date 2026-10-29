//! Cold-start repository scanner.
//!
//! Analyzers count. [`promote`] is the only place that turns a count into a
//! rule, and [`scan`] is the only place that writes one.

mod analyzers;
mod parser;
mod promote;
mod query;
mod roles;
mod scan;

pub(crate) use analyzers::python;
pub use analyzers::{
    analyze_source, Count, Dimension, GoTally, PythonTally, RustTally, ScriptTally, Tally,
};
pub(crate) use analyzers::{decorator_name, is_custom_error, is_framework_decorator};
pub use parser::{content_hash, parse, Family, SupportedLanguage};
pub use promote::{
    observations, opposite_instruction, MIN_FILES, MIN_RATIO, PREFER_ARROW_FUNCTIONS,
    PREFER_CUSTOM_DECORATORS, PREFER_CUSTOM_ERRORS, PREFER_DEEP_RELATIVE, PREFER_DEFAULT_EXPORTS,
    PREFER_FUNCTION_DECLARATIONS, PREFER_GUARD_CLAUSES, PREFER_INTERFACE, PREFER_NAMED_EXPORTS,
    PREFER_PATH_ALIASES, PREFER_TYPE_ALIAS, PY_PREFER_ABSOLUTE_IMPORTS, PY_PREFER_ANNOTATIONS,
    PY_PREFER_CUSTOM_DECORATORS, PY_PREFER_CUSTOM_EXCEPTIONS, PY_PREFER_GUARD_CLAUSES,
    PY_PREFER_LOGGING, PY_PREFER_RELATIVE_IMPORTS,
};
pub(crate) use query::{
    import_locals, is_package_specifier, matches, node_text, unquote, ScriptQuery,
};
pub use roles::is_test_or_script;
pub(crate) use roles::test_file_pattern;
pub use scan::{scan, stored_tally, ScanReport};
