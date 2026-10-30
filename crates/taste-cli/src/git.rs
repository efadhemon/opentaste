//! The few `git` invocations OpenTaste needs, through the `git` binary.
//!
//! Shelling out keeps libgit2 out of the build. Every call runs in the
//! project root, so paths come back relative to it.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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

/// What a commit changed, relative to its first parent.
#[derive(Debug, Default)]
pub struct Changes {
    /// Files added or modified, with whether each was added. Deletions teach
    /// nothing.
    pub changes: Vec<(PathBuf, bool)>,
    /// Every path the commit touched, deletions included.
    pub touched: usize,
}

/// What `revision` changed against its first parent (or against nothing, for
/// a root commit), relative to the project root. A merge lists nothing.
pub fn changed_files(root: &Path, revision: &str) -> Changes {
    let Some(raw) = output(
        root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-commit-id",
            "--name-status",
            "--no-renames",
            "--relative",
            "--root",
            revision,
        ],
    ) else {
        return Changes::default();
    };
    let fields: Vec<&[u8]> = raw
        .split(|byte| *byte == 0)
        .filter(|f| !f.is_empty())
        .collect();
    let mut changes = Changes::default();
    for pair in fields.chunks(2) {
        let [status, path] = pair else {
            continue;
        };
        changes.touched += 1;
        let added = match status.first() {
            Some(b'A') => true,
            Some(b'M') => false,
            _ => continue,
        };
        let path = PathBuf::from(String::from_utf8_lossy(path).into_owned());
        changes.changes.push((path, added));
    }
    changes
}

/// One long-running `git cat-file --batch`, so reading a thousand blobs costs
/// one process instead of a thousand `git show`s.
pub struct Blobs {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Blobs {
    pub fn open(root: &Path) -> Option<Self> {
        let mut child = Command::new("git")
            .args(["cat-file", "--batch"])
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let input = child.stdin.take()?;
        let output = BufReader::new(child.stdout.take()?);
        Some(Blobs {
            child,
            input,
            output,
        })
    }

    /// The bytes of `path` at `revision`, or `None` when it does not exist
    /// there or is not a file.
    pub fn read(&mut self, revision: &str, path: &Path) -> Option<Vec<u8>> {
        let path = path.to_string_lossy().replace('\\', "/");
        writeln!(self.input, "{revision}:./{path}").ok()?;
        self.input.flush().ok()?;
        let mut header = String::new();
        self.output.read_line(&mut header).ok()?;
        let mut parts = header.split_whitespace();
        let (_oid, kind, size) = (parts.next()?, parts.next()?, parts.next());
        // `<name> missing` has no size; anything else is followed by its bytes.
        let size: usize = size?.parse().ok()?;
        let mut bytes = vec![0; size + 1];
        self.output.read_exact(&mut bytes).ok()?;
        bytes.pop();
        (kind == "blob").then_some(bytes)
    }
}

impl Drop for Blobs {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
