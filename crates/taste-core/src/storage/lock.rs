//! Advisory cross-process lock over the rule set.
//!
//! The MCP server, the git `post-commit` hook, the file watcher and an
//! interactive CLI session can all be running at once, and all of them perform
//! read-modify-write cycles over `.taste/rules.json` and the candidate cache.
//! Holding this lock for the duration of such a cycle is what keeps two
//! concurrent promotions from clobbering each other.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::storage::atomic::ensure_parent_dir;

/// An exclusive hold on the OpenTaste lock file, released on drop.
///
/// The lock is advisory: it only excludes other processes that also acquire it.
/// Nothing stops a developer from editing `.taste/rules.json` by hand, which is
/// intentional, since the file is meant to be reviewable and editable.
#[derive(Debug)]
pub struct TasteLock {
    path: PathBuf,
    file: File,
}

impl TasteLock {
    /// Block until the lock is acquired.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let file = open_lock_file(path)?;
        file.lock().map_err(|source| Error::Lock {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(TasteLock {
            path: path.to_path_buf(),
            file,
        })
    }

    /// Acquire the lock, or return `None` if another process holds it.
    pub fn try_acquire(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        let file = open_lock_file(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(TasteLock {
                path: path.to_path_buf(),
                file,
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(Error::Lock {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Path of the lock file being held.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TasteLock {
    fn drop(&mut self) {
        // Closing the handle releases the lock regardless, so an explicit
        // failure here is not worth propagating out of a destructor.
        let _ = self.file.unlock();
    }
}

fn open_lock_file(path: &Path) -> Result<File> {
    ensure_parent_dir(path)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquiring_creates_the_lock_file_and_its_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".taste").join(".lock");

        let lock = TasteLock::acquire(&path).unwrap();
        assert!(path.exists());
        assert_eq!(lock.path(), path);
    }

    #[test]
    fn a_second_attempt_is_refused_while_held_and_succeeds_once_released() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".lock");

        let held = TasteLock::acquire(&path).unwrap();
        assert!(
            TasteLock::try_acquire(&path).unwrap().is_none(),
            "lock should be refused while another guard holds it"
        );

        drop(held);
        assert!(TasteLock::try_acquire(&path).unwrap().is_some());
    }
}
