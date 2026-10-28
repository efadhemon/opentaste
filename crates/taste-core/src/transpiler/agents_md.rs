//! `AGENTS.md` and `.github/copilot-instructions.md`, both updated by
//! marker-block injection.
//!
//! These are two files rather than one because different tools look in
//! different places for the same information; the block written to each is
//! identical.

use std::path::PathBuf;

use crate::config::Paths;
use crate::error::Result;
use crate::models::TasteRule;
use crate::transpiler::{render, SyncReport};

/// Both files this target maintains.
pub fn targets(paths: &Paths) -> [PathBuf; 2] {
    [paths.agents_md(), paths.copilot_instructions()]
}

/// Update the OpenTaste block in `AGENTS.md` and the Copilot instructions.
pub fn sync(paths: &Paths, active: &[TasteRule], report: &mut SyncReport) -> Result<()> {
    for path in targets(paths) {
        crate::transpiler::sync_marker_target(path, active, report)?;
    }
    Ok(())
}

/// The body this target injects.
pub fn render_body(active: &[TasteRule]) -> String {
    render::sections(active)
}
