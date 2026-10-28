//! `.windsurfrules`, updated by marker-block injection.
//!
//! Windsurf reads this file as plain text rather than markdown, but the HTML
//! comment markers are inert either way, and keeping the format identical to
//! the other targets means one rendering path rather than two.

use crate::config::Paths;
use crate::error::Result;
use crate::models::TasteRule;
use crate::transpiler::{render, SyncReport};

/// Update the OpenTaste block in `.windsurfrules`.
pub fn sync(paths: &Paths, active: &[TasteRule], report: &mut SyncReport) -> Result<()> {
    crate::transpiler::sync_marker_target(paths.windsurf_rules(), active, report)
}

/// The body this target injects.
pub fn render_body(active: &[TasteRule]) -> String {
    render::sections(active)
}
