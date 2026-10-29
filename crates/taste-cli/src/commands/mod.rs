//! One module per subcommand.

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

use taste_core::Outcome;

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
