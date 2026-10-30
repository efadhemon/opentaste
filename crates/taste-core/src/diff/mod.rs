//! Structural diff between an AI edit and the developer's correction.
//!
//! Snippets leave this module only through [`scrub`]. Detectors return raw
//! text; [`structural::analyze`] redacts it before the observation is visible
//! to callers.

mod contradiction;
mod go;
mod python;
mod rust;
mod scrubber;
mod structural;

pub use contradiction::contradictions;
pub use scrubber::scrub;
pub use structural::{analyze, analyze_in, PREFER_LOGGER};
