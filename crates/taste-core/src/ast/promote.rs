//! Turn a repository tally into observations.
//!
//! This is the only place a count becomes a rule. A dimension is promoted when
//! one side is at least [`MIN_RATIO`] of the occurrences and at least
//! [`MIN_FILES`] files contributed. Symmetric choices (aliases versus relative
//! imports, `interface` versus `type`) promote whichever side wins. Asymmetric
//! ones only promote the non-default side: a codebase that mostly uses
//! `if/else`, framework decorators, or bare `Error` is not expressing a taste,
//! and a rule that told the editor to keep doing the unremarkable thing would
//! be a wrong rule.

use crate::models::{RuleCategory, RuleSource};
use crate::storage::{validate_scope, Observation};

use super::{Family, GoTally, PythonTally, RustTally, ScriptTally, Tally};

/// Minimum share of occurrences the winning side must hold.
pub const MIN_RATIO: f32 = 0.80;
/// Minimum number of files that must have contributed to the dimension.
pub const MIN_FILES: usize = 10;

pub const PREFER_PATH_ALIASES: &str =
    "Prefer path aliases (`@/` or `~/`) over deep relative imports (`../../`).";
pub const PREFER_DEEP_RELATIVE: &str =
    "Prefer deep relative imports (`../../`) over path aliases (`@/` or `~/`).";
pub const PREFER_NAMED_EXPORTS: &str = "Prefer named exports over default exports.";
pub const PREFER_DEFAULT_EXPORTS: &str = "Prefer default exports over named exports.";
pub const PREFER_INTERFACE: &str =
    "Prefer `interface` declarations over `type` aliases for object types.";
pub const PREFER_TYPE_ALIAS: &str =
    "Prefer `type` aliases over `interface` declarations for object types.";
pub const PREFER_FUNCTION_DECLARATIONS: &str =
    "Prefer `function` declarations over arrow functions assigned to a const.";
pub const PREFER_ARROW_FUNCTIONS: &str =
    "Prefer arrow functions (`const name = () =>`) over `function` declarations.";
pub const PREFER_GUARD_CLAUSES: &str =
    "Prefer early-return guard clauses over nested if/else blocks.";
pub const PREFER_CUSTOM_DECORATORS: &str =
    "Prefer project-specific decorators over framework decorators such as `@Entity` and `@Injectable`.";
pub const PREFER_CUSTOM_ERRORS: &str = "Prefer custom error classes over throwing bare `Error`.";

pub const TS_STRICT_NULL: &str = "This code compiles with `strictNullChecks`: handle `null` and `undefined` explicitly; do not silence them with the non-null assertion `!`.";
/// Confidence of a rule read from compiler settings: the compiler states it,
/// so it is trusted like a developer saying it outright.
pub const COMPILER_CONFIDENCE: f32 = 0.95;

pub const PY_PREFER_ABSOLUTE_IMPORTS: &str =
    "In Python, prefer absolute imports of the project's own package (`from app.users import User`) over relative imports (`from .users import User`).";
pub const PY_PREFER_RELATIVE_IMPORTS: &str =
    "In Python, prefer relative imports within the package (`from .users import User`) over absolute imports of it (`from app.users import User`).";
pub const PY_PREFER_CUSTOM_EXCEPTIONS: &str =
    "In Python, raise the project's own exception classes instead of bare `Exception`.";
pub const PY_PREFER_LOGGING: &str =
    "In Python library code, log through the `logging` module (a module-level `logger`) instead of `print()`.";
pub const PY_PREFER_GUARD_CLAUSES: &str =
    "In Python, prefer early `return` or `raise` guard clauses over nested if/else blocks.";
pub const PY_PREFER_ANNOTATIONS: &str =
    "In Python, annotate every function's parameters and return type.";
pub const PY_PREFER_CUSTOM_DECORATORS: &str =
    "In Python, prefer the project's own decorators over framework decorators.";

pub const RS_PREFER_CRATE_PATHS: &str =
    "In Rust, prefer `crate::` paths over `super::` paths in `use` declarations.";
pub const RS_PREFER_SUPER_PATHS: &str =
    "In Rust, prefer `super::` paths over `crate::` paths in `use` declarations.";
pub const RS_PREFER_QUESTION_MARK: &str =
    "In Rust library code, propagate errors with `?` instead of `.unwrap()` or `.expect()`.";
pub const RS_PREFER_LOG_MACROS: &str =
    "In Rust library code, log with `tracing` or `log` macros instead of `println!` or `eprintln!`.";
pub const RS_PREFER_GUARD_CLAUSES: &str =
    "In Rust, prefer early `return` and `let … else` guards over nested if/else blocks.";

pub const GO_PREFER_WRAPPED_ERRORS: &str =
    "In Go, wrap errors from other packages with context (`fmt.Errorf(\"…: %w\", err)`) instead of returning them bare.";
pub const GO_PREFER_STRUCTURED_LOGGING: &str =
    "In Go library code, log through a structured logger (`log/slog`, zap or zerolog) instead of `fmt.Println` or `log.Printf`.";
pub const GO_PREFER_GUARD_CLAUSES: &str =
    "In Go, prefer early returns over if/else blocks, beyond the usual `if err != nil` checks.";

const SCRIPT_SCOPE: &str = Family::Script.scope();
const PYTHON_SCOPE: &str = Family::Python.scope();
const RUST_SCOPE: &str = Family::Rust.scope();
const GO_SCOPE: &str = Family::Go.scope();
const TYPESCRIPT_SCOPE: &str = "**/*.{ts,tsx}";

/// Observations the tally is strong enough to support, in a stable order.
pub fn observations(tally: &Tally) -> Vec<Observation> {
    let mut found = Vec::new();
    let script = &tally.script;
    push(&mut found, import_paths(script));
    push(&mut found, exports(script));
    push(&mut found, type_constructs(script));
    push(&mut found, function_syntax(script));
    push(&mut found, control_flow(script));
    push(&mut found, decorators(script));
    push(&mut found, errors(script));
    let others = python(&tally.python)
        .into_iter()
        .chain(rust(&tally.rust))
        .chain(go(&tally.go));
    for observation in others {
        push(&mut found, Some(observation));
    }
    found
}

/// Go's dimensions, none of them symmetric.
fn go(tally: &GoTally) -> Vec<Observation> {
    [
        (
            tally.errors,
            RuleCategory::ErrorHandling,
            GO_PREFER_WRAPPED_ERRORS,
        ),
        (
            tally.logging,
            RuleCategory::CodeStyle,
            GO_PREFER_STRUCTURED_LOGGING,
        ),
        (
            tally.control_flow,
            RuleCategory::CodeStyle,
            GO_PREFER_GUARD_CLAUSES,
        ),
    ]
    .into_iter()
    .filter_map(|(count, category, instruction)| {
        choose(count, false).map(|(_, ratio)| observed(category, GO_SCOPE, instruction, ratio))
    })
    .collect()
}

/// Rust's dimensions. Only `use` paths are symmetric.
fn rust(tally: &RustTally) -> Vec<Observation> {
    let mut found = Vec::new();
    if let Some((left_wins, ratio)) = choose(tally.imports, true) {
        let instruction = if left_wins {
            RS_PREFER_CRATE_PATHS
        } else {
            RS_PREFER_SUPER_PATHS
        };
        found.push(observed(
            RuleCategory::Imports,
            RUST_SCOPE,
            instruction,
            ratio,
        ));
    }
    for (count, category, instruction) in [
        (
            tally.errors,
            RuleCategory::ErrorHandling,
            RS_PREFER_QUESTION_MARK,
        ),
        (tally.logging, RuleCategory::CodeStyle, RS_PREFER_LOG_MACROS),
        (
            tally.control_flow,
            RuleCategory::CodeStyle,
            RS_PREFER_GUARD_CLAUSES,
        ),
    ] {
        if let Some((_, ratio)) = choose(count, false) {
            found.push(observed(category, RUST_SCOPE, instruction, ratio));
        }
    }
    found
}

/// Python's dimensions. Only imports are symmetric: a codebase that mostly
/// prints, raises `Exception`, nests `else` or leaves signatures bare is not
/// expressing a taste.
fn python(tally: &PythonTally) -> Vec<Observation> {
    let mut found = Vec::new();
    if let Some((left_wins, ratio)) = choose(tally.imports, true) {
        let instruction = if left_wins {
            PY_PREFER_ABSOLUTE_IMPORTS
        } else {
            PY_PREFER_RELATIVE_IMPORTS
        };
        found.push(observed(
            RuleCategory::Imports,
            PYTHON_SCOPE,
            instruction,
            ratio,
        ));
    }
    for (count, category, instruction) in [
        (
            tally.errors,
            RuleCategory::ErrorHandling,
            PY_PREFER_CUSTOM_EXCEPTIONS,
        ),
        (tally.logging, RuleCategory::CodeStyle, PY_PREFER_LOGGING),
        (
            tally.control_flow,
            RuleCategory::CodeStyle,
            PY_PREFER_GUARD_CLAUSES,
        ),
        (
            tally.annotations,
            RuleCategory::Types,
            PY_PREFER_ANNOTATIONS,
        ),
        (
            tally.decorators,
            RuleCategory::Architecture,
            PY_PREFER_CUSTOM_DECORATORS,
        ),
    ] {
        if let Some((_, ratio)) = choose(count, false) {
            found.push(observed(category, PYTHON_SCOPE, instruction, ratio));
        }
    }
    found
}

fn push(out: &mut Vec<Observation>, observation: Option<Observation>) {
    if let Some(observation) = observation {
        if validate_scope(&observation.scope).is_ok() {
            out.push(observation);
        }
    }
}

fn import_paths(tally: &ScriptTally) -> Option<Observation> {
    let (left_wins, ratio) = choose(tally.import_paths, true)?;
    let instruction = if left_wins {
        PREFER_PATH_ALIASES
    } else {
        PREFER_DEEP_RELATIVE
    };
    Some(observed(
        RuleCategory::Imports,
        SCRIPT_SCOPE,
        instruction,
        ratio,
    ))
}

fn exports(tally: &ScriptTally) -> Option<Observation> {
    let (left_wins, ratio) = choose(tally.exports, true)?;
    let instruction = if left_wins {
        PREFER_NAMED_EXPORTS
    } else {
        PREFER_DEFAULT_EXPORTS
    };
    Some(observed(
        RuleCategory::Imports,
        SCRIPT_SCOPE,
        instruction,
        ratio,
    ))
}

fn type_constructs(tally: &ScriptTally) -> Option<Observation> {
    let (left_wins, ratio) = choose(tally.type_constructs, true)?;
    let instruction = if left_wins {
        PREFER_INTERFACE
    } else {
        PREFER_TYPE_ALIAS
    };
    Some(observed(
        RuleCategory::Types,
        TYPESCRIPT_SCOPE,
        instruction,
        ratio,
    ))
}

fn function_syntax(tally: &ScriptTally) -> Option<Observation> {
    let (left_wins, ratio) = choose(tally.function_syntax, true)?;
    let instruction = if left_wins {
        PREFER_FUNCTION_DECLARATIONS
    } else {
        PREFER_ARROW_FUNCTIONS
    };
    Some(observed(
        RuleCategory::CodeStyle,
        SCRIPT_SCOPE,
        instruction,
        ratio,
    ))
}

fn control_flow(tally: &ScriptTally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.control_flow, false)?;
    Some(observed(
        RuleCategory::CodeStyle,
        SCRIPT_SCOPE,
        PREFER_GUARD_CLAUSES,
        ratio,
    ))
}

fn decorators(tally: &ScriptTally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.decorators, false)?;
    Some(observed(
        RuleCategory::Architecture,
        SCRIPT_SCOPE,
        PREFER_CUSTOM_DECORATORS,
        ratio,
    ))
}

fn errors(tally: &ScriptTally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.errors, false)?;
    Some(observed(
        RuleCategory::ErrorHandling,
        SCRIPT_SCOPE,
        PREFER_CUSTOM_ERRORS,
        ratio,
    ))
}

/// The strict-null rule for each `tsconfig.json` scope that compiles with it.
pub fn strict_null_observations(scopes: &[String]) -> Vec<Observation> {
    scopes
        .iter()
        .filter(|scope| validate_scope(scope).is_ok())
        .map(|scope| {
            observed(
                RuleCategory::Types,
                scope,
                TS_STRICT_NULL,
                COMPILER_CONFIDENCE,
            )
        })
        .collect()
}

fn observed(category: RuleCategory, scope: &str, instruction: &str, ratio: f32) -> Observation {
    Observation::new(category, scope, RuleSource::AstScan, instruction).with_confidence(ratio)
}

/// `(left_wins, ratio)` when a side clears both floors.
///
/// `allow_right` is false for dimensions where the right-hand side is the
/// unremarkable default and must not become a rule.
fn choose(count: super::Count, allow_right: bool) -> Option<(bool, f32)> {
    let total = count.left.saturating_add(count.right);
    if total == 0 || (count.files as usize) < MIN_FILES {
        return None;
    }
    if meets(count.left, total) {
        return Some((true, ratio(count.left, total)));
    }
    if allow_right && meets(count.right, total) {
        return Some((false, ratio(count.right, total)));
    }
    None
}

/// `part / total >= 0.80`, compared in integers so a ratio of exactly 0.80 promotes.
fn meets(part: u32, total: u32) -> bool {
    u64::from(part) * 100 >= u64::from(total) * 80
}

fn ratio(part: u32, total: u32) -> f32 {
    part as f32 / total as f32
}

/// The other side of a symmetric choice, when `instruction` is one we emit.
///
/// A later scan that flips the ratio must not install the opposite rule beside
/// the one already in the file. The two would contradict each other.
pub fn opposite_instruction(instruction: &str) -> Option<&'static str> {
    Some(match instruction {
        PREFER_PATH_ALIASES => PREFER_DEEP_RELATIVE,
        PREFER_DEEP_RELATIVE => PREFER_PATH_ALIASES,
        PREFER_NAMED_EXPORTS => PREFER_DEFAULT_EXPORTS,
        PREFER_DEFAULT_EXPORTS => PREFER_NAMED_EXPORTS,
        PREFER_INTERFACE => PREFER_TYPE_ALIAS,
        PREFER_TYPE_ALIAS => PREFER_INTERFACE,
        PREFER_FUNCTION_DECLARATIONS => PREFER_ARROW_FUNCTIONS,
        PREFER_ARROW_FUNCTIONS => PREFER_FUNCTION_DECLARATIONS,
        PY_PREFER_ABSOLUTE_IMPORTS => PY_PREFER_RELATIVE_IMPORTS,
        PY_PREFER_RELATIVE_IMPORTS => PY_PREFER_ABSOLUTE_IMPORTS,
        RS_PREFER_CRATE_PATHS => RS_PREFER_SUPER_PATHS,
        RS_PREFER_SUPER_PATHS => RS_PREFER_CRATE_PATHS,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Count, ScriptTally, Tally};

    fn files(left: u32, right: u32, n: u32) -> Count {
        Count {
            left,
            right,
            files: n,
        }
    }

    #[test]
    fn a_nine_of_ten_import_ratio_promotes_and_six_of_ten_does_not() {
        let strong = Tally {
            script: ScriptTally {
                import_paths: files(9, 1, 10),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        let promoted = observations(&strong);
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].instruction, PREFER_PATH_ALIASES);
        assert_eq!(promoted[0].confidence_override, Some(0.90));
        assert_eq!(promoted[0].source, RuleSource::AstScan);

        let weak = Tally {
            script: ScriptTally {
                import_paths: files(6, 4, 10),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        assert!(observations(&weak).is_empty());
    }

    #[test]
    fn nine_files_do_not_clear_the_support_floor() {
        let tally = Tally {
            script: ScriptTally {
                import_paths: files(9, 0, 9),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        assert!(observations(&tally).is_empty());
    }

    #[test]
    fn an_even_split_promotes_nothing() {
        let tally = Tally {
            script: ScriptTally {
                type_constructs: files(10, 10, 20),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        assert!(observations(&tally).is_empty());
    }

    #[test]
    fn guard_clauses_promote_only_when_guards_dominate() {
        let guards = Tally {
            script: ScriptTally {
                control_flow: files(9, 1, 10),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        assert_eq!(observations(&guards)[0].instruction, PREFER_GUARD_CLAUSES);

        let nested = Tally {
            script: ScriptTally {
                control_flow: files(1, 9, 10),
                ..ScriptTally::default()
            },
            ..Tally::default()
        };
        assert!(
            observations(&nested).is_empty(),
            "a nested-if majority is not a taste to impose"
        );
    }

    #[test]
    fn the_same_dimension_in_two_languages_gets_two_rule_ids() {
        let ids: Vec<_> = Family::ALL
            .iter()
            .map(|family| {
                crate::models::rule_id(
                    RuleCategory::CodeStyle,
                    family.scope(),
                    PREFER_GUARD_CLAUSES,
                )
            })
            .collect();
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), Family::ALL.len(), "{ids:?}");
    }

    #[test]
    fn python_counts_promote_python_rules_scoped_to_python_files() {
        let tally = Tally {
            python: PythonTally {
                imports: files(2, 18, 12),
                logging: files(1, 9, 10),
                annotations: files(45, 5, 10),
                ..PythonTally::default()
            },
            ..Tally::default()
        };
        let promoted = observations(&tally);
        let instructions: Vec<_> = promoted.iter().map(|o| o.instruction.as_str()).collect();
        assert_eq!(
            instructions,
            vec![PY_PREFER_RELATIVE_IMPORTS, PY_PREFER_ANNOTATIONS]
        );
        assert!(promoted.iter().all(|o| o.scope == "**/*.py"));
        assert_eq!(
            opposite_instruction(PY_PREFER_RELATIVE_IMPORTS),
            Some(PY_PREFER_ABSOLUTE_IMPORTS)
        );
    }

    #[test]
    fn a_print_majority_is_not_a_rule() {
        let tally = Tally {
            python: PythonTally {
                logging: files(1, 30, 12),
                errors: files(0, 20, 12),
                ..PythonTally::default()
            },
            ..Tally::default()
        };
        assert!(observations(&tally).is_empty());
    }

    #[test]
    fn rust_counts_promote_rust_rules_scoped_to_rust_files() {
        let tally = Tally {
            rust: RustTally {
                imports: files(40, 2, 15),
                errors: files(90, 10, 12),
                logging: files(2, 40, 10),
                ..RustTally::default()
            },
            ..Tally::default()
        };
        let promoted = observations(&tally);
        let instructions: Vec<_> = promoted.iter().map(|o| o.instruction.as_str()).collect();
        assert_eq!(
            instructions,
            vec![RS_PREFER_CRATE_PATHS, RS_PREFER_QUESTION_MARK]
        );
        assert!(promoted.iter().all(|o| o.scope == "**/*.rs"));
        assert_eq!(
            opposite_instruction(RS_PREFER_SUPER_PATHS),
            Some(RS_PREFER_CRATE_PATHS)
        );
    }

    #[test]
    fn go_counts_promote_go_rules_scoped_to_go_files() {
        let tally = Tally {
            go: GoTally {
                errors: files(45, 5, 12),
                logging: files(3, 30, 10),
                control_flow: files(9, 1, 9),
            },
            ..Tally::default()
        };
        let promoted = observations(&tally);
        let instructions: Vec<_> = promoted.iter().map(|o| o.instruction.as_str()).collect();
        assert_eq!(instructions, vec![GO_PREFER_WRAPPED_ERRORS]);
        assert_eq!(promoted[0].scope, "**/*.go");
    }
}
