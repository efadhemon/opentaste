//! `CLAUDE.md`, updated by marker-block injection.

use crate::config::Paths;
use crate::error::Result;
use crate::models::TasteRule;
use crate::transpiler::{marker, render, SyncReport};

/// Update the OpenTaste block in `CLAUDE.md`, leaving the rest of it alone.
pub fn sync(paths: &Paths, active: &[TasteRule], report: &mut SyncReport) -> Result<()> {
    crate::transpiler::sync_marker_target(paths.claude_md(), active, report)
}

/// The body this target injects, exposed for tests and `taste status`.
pub fn render_body(active: &[TasteRule]) -> String {
    render::sections(active)
}

/// Whether `CLAUDE.md` currently carries an OpenTaste block.
pub fn has_block(paths: &Paths) -> Result<bool> {
    let contents = crate::storage::atomic::read_to_string_opt(&paths.claude_md())?;
    Ok(contents.as_deref().map(marker::has_block).unwrap_or(false))
}
