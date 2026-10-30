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
pub mod tsconfig;

pub(crate) use analyzers::go;
pub(crate) use analyzers::python;
pub(crate) use analyzers::rust;
pub(crate) use analyzers::validation;
pub use analyzers::validation::Library;
pub use analyzers::{
    analyze_source, Count, Dimension, GoTally, PythonTally, RustTally, ScriptTally, Tally,
};
pub(crate) use analyzers::{decorator_name, is_custom_error, is_framework_decorator};
pub use parser::{content_hash, parse, Family, SupportedLanguage};
pub use promote::{
    conflicting_instructions, observations, opposite_instruction, strict_null_observations,
    validate_with, COMPILER_CONFIDENCE, GO_PREFER_GUARD_CLAUSES, GO_PREFER_STRUCTURED_LOGGING,
    GO_PREFER_WRAPPED_ERRORS, MIN_FILES, MIN_RATIO, PREFER_ARROW_FUNCTIONS,
    PREFER_CUSTOM_DECORATORS, PREFER_CUSTOM_ERRORS, PREFER_DEEP_RELATIVE, PREFER_DEFAULT_EXPORTS,
    PREFER_FUNCTION_DECLARATIONS, PREFER_GUARD_CLAUSES, PREFER_INTERFACE, PREFER_NAMED_EXPORTS,
    PREFER_PATH_ALIASES, PREFER_TYPE_ALIAS, PY_PREFER_ABSOLUTE_IMPORTS, PY_PREFER_ANNOTATIONS,
    PY_PREFER_CUSTOM_DECORATORS, PY_PREFER_CUSTOM_EXCEPTIONS, PY_PREFER_GUARD_CLAUSES,
    PY_PREFER_LOGGING, PY_PREFER_RELATIVE_IMPORTS, RS_PREFER_CRATE_PATHS, RS_PREFER_GUARD_CLAUSES,
    RS_PREFER_LOG_MACROS, RS_PREFER_QUESTION_MARK, RS_PREFER_SUPER_PATHS, TS_PREFER_Z_INFER,
    TS_STRICT_NULL, VALIDATE_WITH_CLASS_VALIDATOR, VALIDATE_WITH_IO_TS, VALIDATE_WITH_JOI,
    VALIDATE_WITH_VALIBOT, VALIDATE_WITH_YUP, VALIDATE_WITH_ZOD,
};
pub(crate) use query::{
    import_locals, is_package_specifier, matches, node_text, unquote, ScriptQuery,
};
pub use roles::is_test_or_script;
pub(crate) use roles::test_file_pattern;
pub use scan::{scan, stored_tally, ScanReport};
