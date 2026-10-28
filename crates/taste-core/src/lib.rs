//! Core engine for OpenTaste.
//!
//! The crate is organised around three concerns:
//!
//! - [`models`] defines the rule shapes shared by every other layer.
//! - [`storage`] owns persistence, with [`storage::RuleStore`] as the single
//!   mutation entry point over the git-tracked JSON file and the local SQLite
//!   candidate cache.
//! - [`transpiler`] compiles confirmed rules into editor instruction files.
//!
//! [`scorer`] holds the pure confidence math and deduplication helpers that
//! [`storage::RuleStore`] drives.

pub mod config;
pub mod error;
pub mod models;
pub mod scorer;
pub mod storage;
pub mod transpiler;

pub use error::{Error, Result};
pub use models::{CandidateRule, ProjectContext, RuleCategory, RuleSource, RulesFile, TasteRule};
pub use storage::{Observation, Outcome, RuleStore};
