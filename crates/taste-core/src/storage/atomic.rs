//! Crash-safe file writes.
//!
//! Every file OpenTaste generates is written to a sibling temporary file and
//! then renamed over the target. A `rename` within one directory is atomic on
//! both Unix and Windows, so a reader never observes a half-written
//! `CLAUDE.md` and an interrupted sync never truncates one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Suffix for the temporary file used during an atomic write.
const TEMP_SUFFIX: &str = ".opentaste-tmp";

/// Create `path`'s parent directory if it does not already exist.
pub fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
    }
    Ok(())
}

/// Write `contents` to `path`, replacing it atomically.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    ensure_parent_dir(path)?;

    let temp_path = temp_path_for(path);
    {
        let mut file = fs::File::create(&temp_path).map_err(|e| Error::io(&temp_path, e))?;
        file.write_all(contents)
            .map_err(|e| Error::io(&temp_path, e))?;
        // Flush to disk before the rename so a power loss cannot leave the
        // renamed file present but empty.
        file.sync_all().map_err(|e| Error::io(&temp_path, e))?;
    }

    fs::rename(&temp_path, path).map_err(|e| {
        // Leaving a stray temp file behind would be worse than the original
        // failure, so clean up before reporting.
        let _ = fs::remove_file(&temp_path);
        Error::io(path, e)
    })
}

/// Write `contents` only if they differ from what is already on disk.
///
/// Returns `true` when the file was written. Skipping no-op writes keeps mtimes
/// stable, which matters because the file watcher would otherwise treat the
/// transpiler's own output as a change worth reacting to.
pub fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    if let Ok(existing) = fs::read(path) {
        if existing == contents {
            return Ok(false);
        }
    }
    write_atomic(path, contents)?;
    Ok(true)
}

/// Read a file to a string, returning `None` when it does not exist.
pub fn read_to_string_opt(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::io(path, e)),
    }
}

/// Remove a file, treating an already-absent file as success.
///
/// Returns `true` when a file was actually removed.
pub fn remove_if_present(path: &Path) -> Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::io(path, e)),
    }
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(TEMP_SUFFIX);
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_creates_missing_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("a").join("b").join("file.md");

        write_atomic(&target, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
    }

    #[test]
    fn write_atomic_leaves_no_temporary_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("file.md");

        write_atomic(&target, b"one").unwrap();
        write_atomic(&target, b"two").unwrap();

        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["file.md"]);
        assert_eq!(fs::read_to_string(&target).unwrap(), "two");
    }

    #[test]
    fn write_if_changed_skips_identical_content() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("file.md");

        assert!(write_if_changed(&target, b"same").unwrap());
        assert!(!write_if_changed(&target, b"same").unwrap());
        assert!(write_if_changed(&target, b"different").unwrap());
    }

    #[test]
    fn read_to_string_opt_reports_absence_rather_than_failing() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope.md");
        assert_eq!(read_to_string_opt(&missing).unwrap(), None);

        write_atomic(&missing, b"now here").unwrap();
        assert_eq!(
            read_to_string_opt(&missing).unwrap(),
            Some("now here".to_string())
        );
    }

    #[test]
    fn remove_if_present_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("file.md");
        write_atomic(&target, b"x").unwrap();

        assert!(remove_if_present(&target).unwrap());
        assert!(!remove_if_present(&target).unwrap());
    }
}
