//! One module per subcommand.

pub mod global;
pub mod history;
pub mod init;
pub mod learn;
pub mod learn_delta;
pub mod mcp;
pub mod scan;
pub mod status;
pub mod sync;
pub mod targets;
pub mod toggle;
pub mod watch;

use std::path::Path;

use taste_core::{content_hash, Observation, Outcome};

/// Identifies one observation of one edit to one file, so the editor
/// extension and the post-commit hook can tell they saw the same correction.
/// Built from what the detector extracted, not from the surrounding text,
/// which differs between the extension's block and the committed file.
pub fn delta_key(path: &Path, observation: &Observation) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let parts = [
        path.as_str(),
        observation.category.slug(),
        observation.instruction.as_str(),
        observation.anti_pattern.as_deref().unwrap_or(""),
        observation.preferred_pattern.as_deref().unwrap_or(""),
    ];
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    content_hash(&bytes)
}

/// Attach the developer's personal rules to `store`, so its confirmations are
/// remembered for suggestions and its files are served personal rules.
/// Personal rules are an extra: without a usable home directory every project
/// command still works, and `taste global` says what went wrong.
pub fn with_personal_rules(store: taste_core::RuleStore) -> taste_core::RuleStore {
    let global = taste_core::config::Paths::home()
        .ok()
        .and_then(|home| taste_core::storage::GlobalStore::open(&home).ok());
    match global {
        Some(global) => store.with_global(global),
        None => store,
    }
}

/// Identifies one file's change by its path and both versions, not by commit:
/// an amended or cherry-picked commit carrying the same change matches, and
/// so does the same commit read by the post-commit hook and by history mining.
pub fn change_fingerprint(path: &Path, before: &str, after: &str) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let mut bytes = Vec::with_capacity(path.len() + before.len() + after.len() + 2);
    for part in [path.as_bytes(), before.as_bytes(), after.as_bytes()] {
        bytes.extend_from_slice(part);
        bytes.push(0);
    }
    content_hash(&bytes)
}

/// One line describing what recording an observation did.
pub fn describe_outcome(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Promoted(rule) => format!(
            "confirmed  {} ({:.0}%): {}",
            rule.id,
            rule.confidence * 100.0,
            rule.instruction
        ),
        Outcome::AlreadyConfirmed { id, confidence, .. } => {
            format!("reinforced {id} ({:.0}%)", confidence * 100.0)
        }
        Outcome::Created { id, confidence } => {
            format!("candidate  {id} ({:.0}%)", confidence * 100.0)
        }
        Outcome::Reinforced {
            id,
            confidence,
            occurrences,
        } => format!(
            "candidate  {id} ({:.0}%, seen {occurrences} times)",
            confidence * 100.0
        ),
        Outcome::Conflicts { id, conflicts_with } => {
            format!("skipped    {id}: contradicts confirmed rule {conflicts_with}")
        }
    }
}
