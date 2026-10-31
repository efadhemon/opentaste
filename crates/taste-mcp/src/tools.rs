//! The three tools: read, record and reject a coding preference.
//!
//! Each maps onto one `RuleStore` call. A failure inside a tool, such as an
//! unknown rule id or a bad glob, is a *tool execution* error: it comes back
//! in a successful response with `isError: true`, so the model reads the
//! message and can correct itself. Only an unknown tool name is a protocol
//! error, and that is decided in [`crate::server`].

use std::path::Path;

use serde_json::{json, Value};
use taste_core::models::RuleSource;
use taste_core::storage::validate_scope;
use taste_core::transpiler::SyncReport;
use taste_core::{
    scrub, Observation, Outcome, RuleCategory, RuleStore, SupportedLanguage, TasteRule,
};

pub const GET_CODING_TASTE: &str = "get_coding_taste";
pub const RECORD_CODING_TASTE: &str = "record_coding_taste";
pub const REJECT_CODING_TASTE: &str = "reject_coding_taste";

/// The `tools/list` payload. The names are part of the wire contract: an
/// editor configured against them breaks silently if one is renamed.
pub fn definitions() -> Value {
    let categories: Vec<&str> = RuleCategory::ALL.iter().map(|c| c.slug()).collect();
    json!([
        {
            "name": GET_CODING_TASTE,
            "description": "Get the project's learned coding preferences that apply to a file. \
                Call this before writing or editing a file so your changes match the \
                developer's established style, architecture and naming.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "Path of the file being edited, relative to the project root."
                    }
                },
                "required": ["file_path"]
            }
        },
        {
            "name": RECORD_CODING_TASTE,
            "description": "Call this tool immediately whenever the user corrects your code style, \
                architecture, naming, or explicitly states a coding preference. Record the \
                preference as one short imperative instruction so it is applied to future \
                edits in this project.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "category": {
                        "type": "string",
                        "enum": categories,
                        "description": "Which kind of preference this is."
                    },
                    "scope": {
                        "type": "string",
                        "description": "Glob of the files the preference applies to, such as \
                            `src/**/*.entity.ts`, or `**/*` for the whole project. Prefer the \
                            narrowest glob that fits what the user said."
                    },
                    "instruction": {
                        "type": "string",
                        "description": "The preference as one imperative sentence, such as \
                            \"Use @AutoEntity instead of @Entity.\""
                    },
                    "anti_pattern": {
                        "type": "string",
                        "description": "Optional short snippet of the code the user rejected."
                    },
                    "preferred_pattern": {
                        "type": "string",
                        "description": "Optional short snippet of the code the user wants instead."
                    },
                    "global": {
                        "type": "boolean",
                        "description": "Also keep this as one of the user's personal preferences, \
                            served in their other projects. Only when the user says it is how \
                            they always work, not just this project's convention.",
                        "default": false
                    }
                },
                "required": ["category", "scope", "instruction"]
            }
        },
        {
            "name": REJECT_CODING_TASTE,
            "description": "Stop applying a learned coding preference that the user says is wrong \
                or no longer wanted. Disables the rule by default, which can be undone; pass \
                `delete: true` only when the user explicitly asks for it to be removed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rule_id": {
                        "type": "string",
                        "description": "The rule's id, as returned by get_coding_taste."
                    },
                    "delete": {
                        "type": "boolean",
                        "description": "Remove the rule entirely instead of disabling it.",
                        "default": false
                    }
                },
                "required": ["rule_id"]
            }
        }
    ])
}

/// Run the tool called `name`, or `None` when no such tool exists.
pub fn call(name: &str, arguments: &Value, store: &RuleStore) -> Option<Value> {
    let outcome = match name {
        GET_CODING_TASTE => get(arguments, store),
        RECORD_CODING_TASTE => record(arguments, store),
        REJECT_CODING_TASTE => reject(arguments, store),
        _ => return None,
    };
    Some(match outcome {
        Ok(text) => result(text, false),
        Err(text) => result(text, true),
    })
}

type ToolResult = Result<String, String>;

fn result(text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error,
    })
}

fn get(arguments: &Value, store: &RuleStore) -> ToolResult {
    let file_path = required_str(arguments, "file_path")?;
    let rules = store.rules_for_file(file_path).map_err(failed)?;
    let personal = store.personal_rules_for_file(file_path).map_err(failed)?;
    if rules.is_empty() && personal.is_empty() {
        return Ok(format!("No OpenTaste preferences apply to {file_path}."));
    }
    let total = rules.len() + personal.len();
    let mut text = format!(
        "{} OpenTaste preference{} appl{} to {file_path}:\n",
        total,
        if total == 1 { "" } else { "s" },
        if total == 1 { "ies" } else { "y" },
    );
    for rule in &rules {
        text.push_str(&describe(rule));
    }
    if !personal.is_empty() {
        // After the project's own rules, which win where the two disagree.
        text.push_str(
            "\nPersonal preferences (the user's own, across projects; this project's rules come first):\n",
        );
        for rule in &personal {
            text.push_str(&describe(rule).replacen("\n- [", "\n- (personal) [", 1));
        }
    }
    Ok(text)
}

fn describe(rule: &TasteRule) -> String {
    let mut line = format!(
        "\n- [{}] {}\n  id: {} · scope: {} · confidence: {:.2}\n",
        rule.category.slug(),
        rule.instruction,
        rule.id,
        rule.scope,
        rule.confidence
    );
    if let Some(anti) = &rule.anti_pattern {
        line.push_str(&format!("  avoid:\n{}\n", indent(anti)));
    }
    if let Some(preferred) = &rule.preferred_pattern {
        line.push_str(&format!("  prefer:\n{}\n", indent(preferred)));
    }
    line
}

fn indent(snippet: &str) -> String {
    snippet
        .lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn record(arguments: &Value, store: &RuleStore) -> ToolResult {
    let category_raw = required_str(arguments, "category")?;
    let category: RuleCategory = category_raw.parse().map_err(|_| {
        let valid: Vec<&str> = RuleCategory::ALL.iter().map(|c| c.slug()).collect();
        format!(
            "{category_raw:?} is not a category. Use one of: {}.",
            valid.join(", ")
        )
    })?;
    let scope = required_str(arguments, "scope")?;
    // The one place a glob arrives from outside the codebase.
    validate_scope(scope)
        .map_err(|err| format!("{err}. Pass a valid glob such as `src/**/*.ts`."))?;
    let instruction = required_str(arguments, "instruction")?;
    // Snippets land in the git-tracked rule file, so they get the same secret
    // scrubbing as snippets learned from a diff. The scope's extension picks
    // the grammar; TSX parses JavaScript and TypeScript alike.
    let lang = SupportedLanguage::from_path(Path::new(scope)).unwrap_or(SupportedLanguage::Tsx);
    let anti = optional_str(arguments, "anti_pattern")?.map(|s| scrub(&s, lang));
    let preferred = optional_str(arguments, "preferred_pattern")?.map(|s| scrub(&s, lang));

    let global = match arguments.get("global") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(global)) => *global,
        Some(_) => return Err("`global` must be true or false.".to_string()),
    };

    let observation = Observation::new(category, scope, RuleSource::McpFeedback, instruction)
        .with_patterns(anti, preferred);
    let outcome = store.record_observation(observation).map_err(failed)?;
    let personal = if global {
        make_personal(&outcome, store)?
    } else {
        String::new()
    };
    let summary = match &outcome {
        Outcome::Promoted(rule) => format!(
            "Recorded \"{}\" as rule {} (scope {}, confidence {:.2}).",
            rule.instruction, rule.id, rule.scope, rule.confidence
        ),
        Outcome::AlreadyConfirmed { id, confidence, .. } => format!(
            "This matches existing rule {id}; it was reinforced to confidence {confidence:.2}."
        ),
        Outcome::Created { id, confidence } | Outcome::Reinforced { id, confidence, .. } => {
            format!(
            "Noted as candidate {id} at confidence {confidence:.2}. It is not applied to editor \
             files until it is confirmed."
        )
        }
        Outcome::Conflicts { conflicts_with, .. } => {
            return Err(format!(
                "Not recorded: this is the opposite of existing rule {conflicts_with}. If the \
                 user has changed their mind, call {REJECT_CODING_TASTE} with rule_id \
                 {conflicts_with:?} first, then record this again."
            ))
        }
    };
    let report = store.sync().map_err(failed)?;
    Ok(format!(
        "{summary}{personal}\n{}",
        describe_sync(&report, store)
    ))
}

/// Keep the rule `outcome` confirmed as one of the user's personal rules,
/// without its snippets, which are this project's code.
fn make_personal(outcome: &Outcome, store: &RuleStore) -> Result<String, String> {
    let id = match outcome {
        Outcome::Promoted(rule) => rule.id.clone(),
        Outcome::AlreadyConfirmed { id, .. } => id.clone(),
        _ => return Ok(" It is not confirmed yet, so it was not made personal.".to_string()),
    };
    let Some(global) = store.global() else {
        return Ok(
            " Personal rules are unavailable here (no home directory), so it was \
            kept for this project only."
                .to_string(),
        );
    };
    let rules = store.rules().map_err(failed)?;
    let Some(rule) = rules.find(&id) else {
        return Ok(String::new());
    };
    global
        .add(rule, false, &store.paths().project_root)
        .map_err(failed)?;
    Ok(format!(
        " It is also one of the user's personal rules now ({id})."
    ))
}

fn reject(arguments: &Value, store: &RuleStore) -> ToolResult {
    let rule_id = required_str(arguments, "rule_id")?;
    let delete = match arguments.get("delete") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(delete)) => *delete,
        Some(_) => return Err("`delete` must be true or false.".to_string()),
    };
    let summary = if delete {
        if !store.delete_rule(rule_id).map_err(failed)? {
            return Err(format!("No rule or candidate has id {rule_id:?}."));
        }
        format!("Deleted rule {rule_id}. A rescan will not bring it back.")
    } else {
        if !store.set_enabled(rule_id, false).map_err(failed)? {
            return Err(format!(
                "No confirmed rule has id {rule_id:?}. Candidates are not applied to editor files; \
                 pass `delete: true` to remove one."
            ));
        }
        format!("Disabled rule {rule_id}. It stays in .taste/rules.json and can be re-enabled.")
    };
    let report = store.sync().map_err(failed)?;
    Ok(format!("{summary}\n{}", describe_sync(&report, store)))
}

fn describe_sync(report: &SyncReport, store: &RuleStore) -> String {
    if report.is_empty() {
        return "Editor instruction files were already up to date.".to_string();
    }
    let relative = |path: &std::path::PathBuf| -> String {
        store
            .paths()
            .relativize(Path::new(path))
            .to_string_lossy()
            .replace('\\', "/")
    };
    let mut parts = Vec::new();
    if !report.written.is_empty() {
        let files: Vec<String> = report.written.iter().map(relative).collect();
        parts.push(format!("Updated {}.", files.join(", ")));
    }
    if !report.removed.is_empty() {
        let files: Vec<String> = report.removed.iter().map(relative).collect();
        parts.push(format!("Removed {}.", files.join(", ")));
    }
    parts.join(" ")
}

fn required_str<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    match arguments.get(key) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
        Some(Value::String(_)) => Err(format!("`{key}` must not be empty.")),
        Some(_) => Err(format!("`{key}` must be a string.")),
        None => Err(format!("`{key}` is required.")),
    }
}

fn optional_str(arguments: &Value, key: &str) -> Result<Option<String>, String> {
    match arguments.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(format!("`{key}` must be a string when given.")),
    }
}

fn failed(err: taste_core::Error) -> String {
    format!("OpenTaste could not complete this: {err}")
}
