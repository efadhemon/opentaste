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

use super::Tally;

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

const SCRIPT_SCOPE: &str = "**/*.{ts,tsx,js,jsx,mjs,cjs}";
const TYPESCRIPT_SCOPE: &str = "**/*.{ts,tsx}";

/// Observations the tally is strong enough to support, in a stable order.
pub fn observations(tally: &Tally) -> Vec<Observation> {
    let mut found = Vec::new();
    push(&mut found, import_paths(tally));
    push(&mut found, exports(tally));
    push(&mut found, type_constructs(tally));
    push(&mut found, function_syntax(tally));
    push(&mut found, control_flow(tally));
    push(&mut found, decorators(tally));
    push(&mut found, errors(tally));
    found
}

fn push(out: &mut Vec<Observation>, observation: Option<Observation>) {
    if let Some(observation) = observation {
        if validate_scope(&observation.scope).is_ok() {
            out.push(observation);
        }
    }
}

fn import_paths(tally: &Tally) -> Option<Observation> {
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

fn exports(tally: &Tally) -> Option<Observation> {
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

fn type_constructs(tally: &Tally) -> Option<Observation> {
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

fn function_syntax(tally: &Tally) -> Option<Observation> {
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

fn control_flow(tally: &Tally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.control_flow, false)?;
    Some(observed(
        RuleCategory::CodeStyle,
        SCRIPT_SCOPE,
        PREFER_GUARD_CLAUSES,
        ratio,
    ))
}

fn decorators(tally: &Tally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.decorators, false)?;
    Some(observed(
        RuleCategory::Architecture,
        SCRIPT_SCOPE,
        PREFER_CUSTOM_DECORATORS,
        ratio,
    ))
}

fn errors(tally: &Tally) -> Option<Observation> {
    let (_left_wins, ratio) = choose(tally.errors, false)?;
    Some(observed(
        RuleCategory::ErrorHandling,
        SCRIPT_SCOPE,
        PREFER_CUSTOM_ERRORS,
        ratio,
    ))
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
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Count, Tally};

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
            import_paths: files(9, 1, 10),
            ..Tally::default()
        };
        let promoted = observations(&strong);
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].instruction, PREFER_PATH_ALIASES);
        assert_eq!(promoted[0].confidence_override, Some(0.90));
        assert_eq!(promoted[0].source, RuleSource::AstScan);

        let weak = Tally {
            import_paths: files(6, 4, 10),
            ..Tally::default()
        };
        assert!(observations(&weak).is_empty());
    }

    #[test]
    fn nine_files_do_not_clear_the_support_floor() {
        let tally = Tally {
            import_paths: files(9, 0, 9),
            ..Tally::default()
        };
        assert!(observations(&tally).is_empty());
    }

    #[test]
    fn an_even_split_promotes_nothing() {
        let tally = Tally {
            type_constructs: files(10, 10, 20),
            ..Tally::default()
        };
        assert!(observations(&tally).is_empty());
    }

    #[test]
    fn guard_clauses_promote_only_when_guards_dominate() {
        let guards = Tally {
            control_flow: files(9, 1, 10),
            ..Tally::default()
        };
        assert_eq!(observations(&guards)[0].instruction, PREFER_GUARD_CLAUSES);

        let nested = Tally {
            control_flow: files(1, 9, 10),
            ..Tally::default()
        };
        assert!(
            observations(&nested).is_empty(),
            "a nested-if majority is not a taste to impose"
        );
    }
}
