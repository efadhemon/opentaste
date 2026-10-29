//! The few `git` invocations OpenTaste needs, through the `git` binary.
//!
//! Shelling out keeps libgit2 out of the build. Every call runs in the
//! project root, so paths come back relative to it.

use std::path::Path;
use std::process::{Command, Stdio};

/// Run `git` in `root` and return its stdout, or `None` when git is missing
/// or the command failed.
pub fn output(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let result = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    result.status.success().then_some(result.stdout)
}

/// [`output`] as trimmed UTF-8.
pub fn text(root: &Path, args: &[&str]) -> Option<String> {
    let bytes = output(root, args)?;
    Some(String::from_utf8_lossy(&bytes).trim().to_string())
}

/// Whether `root` is inside a git work tree.
pub fn is_repository(root: &Path) -> bool {
    text(root, &["rev-parse", "--is-inside-work-tree"]).as_deref() == Some("true")
}
