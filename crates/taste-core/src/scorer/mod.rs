//! Pure scoring and similarity logic.
//!
//! Nothing here touches the filesystem; [`crate::storage::RuleStore`] is what
//! drives these functions and persists their results.

pub mod confidence;
pub mod dedup;

pub use confidence::{decay, initial_confidence, is_promotable, reinforce, should_demote};
pub use dedup::{is_duplicate, scope_overlap, token_jaccard};
