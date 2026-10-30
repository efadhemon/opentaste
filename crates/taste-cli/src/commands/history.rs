//! `taste scan --history`: learn from the project's recent commits.
//!
//! A project with years of corrections should not start from nothing, but a
//! wrong detector run over all of history would be wrong once per commit. So
//! the walk is bounded and narrow:
//!
//! - **A window, not all of history.** The last `N` non-merge commits on the
//!   first-parent chain, and for `taste init` no older than 180 days.
//! - **Corrections only.** Code written before a rule existed is not the
//!   developer working against it, so history never applies contradictions.
//! - **Every change once.** Each file's change carries the fingerprint the
//!   post-commit hook uses, so mining twice, or mining a commit the hook also
//!   saw, counts it once. A cursor remembers the newest commit read, and the
//!   next run starts after it.
//!
//! Commits that are not anyone's taste are skipped: those by bots, reverts,
//! and sweeps touching more than [`MAX_FILES`] files (formatting, vendoring,
//! generated code).
//!
//! Analysis is parallel across files; recording is serial and oldest first,
//! through [`RuleStore`]. A file's fingerprint is stored *before* its
//! observations, and the cursor after each commit, so a run that is
//! interrupted resumes where it stopped and at worst misses one file's
//! correction, never counts one twice.

use std::path::{Path, PathBuf};

use chrono::Utc;
use rayon::prelude::*;
use taste_core::{Observation, Outcome, RuleStore, SupportedLanguage};

use crate::commands::{change_fingerprint, delta_key};
use crate::git::{self, Blobs};

/// Commits touching more files than this are sweeps, not corrections.
pub const MAX_FILES: usize = 50;
/// Larger files are skipped, as the scanner skips them.
const MAX_FILE_BYTES: usize = 256 * 1024;

const CURSOR: &str = "history.cursor";
const COMMITS_READ: &str = "history.commits";
const READ_AT: &str = "history.read_at";

/// How far back to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub commits: usize,
    pub days: Option<u32>,
}

impl Window {
    /// What `taste init` reads: 200 commits or 180 days, whichever is fewer.
    pub const INIT: Window = Window {
        commits: 200,
        days: Some(180),
    };

    /// `taste scan --history N`: the last `N` commits, however old.
    pub fn commits(commits: usize) -> Window {
        Window {
            commits,
            days: None,
        }
    }
}

/// One commit from `git log`.
#[derive(Debug, Clone)]
struct Commit {
    sha: String,
    author: String,
    email: String,
    subject: String,
    body: String,
}

/// Why a commit in the window was not learned from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Bot,
    Revert,
    Sweep,
}

/// What one history run did.
#[derive(Debug, Default)]
pub struct HistoryReport {
    /// Commits learned from (or found already learned).
    pub commits: usize,
    pub skipped_bots: usize,
    pub skipped_reverts: usize,
    pub skipped_sweeps: usize,
    /// Outcomes of the corrections history recorded, oldest first.
    pub outcomes: Vec<Outcome>,
    /// Corrections already counted, by the hook, the editor or an earlier run.
    pub already_learned: usize,
}

impl HistoryReport {
    pub fn promoted(&self) -> bool {
        self.outcomes
            .iter()
            .any(|outcome| outcome.promoted().is_some())
    }
}

/// The commits in `window` after the cursor, oldest first.
fn commits_in(root: &Path, window: Window, cursor: Option<&str>) -> Vec<Commit> {
    let count = format!("--max-count={}", window.commits);
    let since = window.days.map(|days| format!("--since={days}.days.ago"));
    // A cursor that is no longer an ancestor (a rebase, a force-push) is
    // ignored; the fingerprints still keep anything already learned from
    // being counted twice.
    let range = cursor
        .filter(|cursor| {
            git::output(root, &["merge-base", "--is-ancestor", cursor, "HEAD"]).is_some()
        })
        .map(|cursor| format!("{cursor}..HEAD"))
        .unwrap_or_else(|| "HEAD".to_string());
    let mut args = vec![
        "log",
        "--first-parent",
        "--no-merges",
        "--format=%H%x00%an%x00%ae%x00%s%x00%b%x1e",
        count.as_str(),
    ];
    if let Some(since) = &since {
        args.push(since);
    }
    args.push(&range);
    let Some(raw) = git::output(root, &args) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&raw);
    let mut commits: Vec<Commit> = text
        .split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').splitn(5, '\0');
            Some(Commit {
                sha: fields.next().filter(|sha| !sha.is_empty())?.to_string(),
                author: fields.next()?.to_string(),
                email: fields.next()?.to_string(),
                subject: fields.next()?.to_string(),
                body: fields.next().unwrap_or("").to_string(),
            })
        })
        .collect();
    commits.reverse();
    commits
}

const BOTS: &[&str] = &["dependabot", "renovate", "github-actions", "[bot]"];

fn skip_reason(commit: &Commit, touched: usize) -> Option<Skip> {
    let who = format!("{} {}", commit.author, commit.email).to_ascii_lowercase();
    if BOTS.iter().any(|bot| who.contains(bot)) {
        return Some(Skip::Bot);
    }
    if commit.subject.starts_with("Revert ") || commit.body.contains("This reverts commit") {
        return Some(Skip::Revert);
    }
    (touched > MAX_FILES).then_some(Skip::Sweep)
}

/// One changed file of one commit, read and fingerprinted.
struct FileChange {
    commit: usize,
    path: PathBuf,
    before: String,
    after: String,
    fingerprint: String,
}

/// Learn from the commits in `window` that have not been read yet.
/// `progress` is called with `(done, total)` as commits are recorded.
pub fn mine(
    store: &RuleStore,
    window: Window,
    progress: &mut dyn FnMut(usize, usize),
) -> anyhow::Result<HistoryReport> {
    let mut report = HistoryReport::default();
    let root = store.paths().project_root.clone();
    if window.commits == 0 || !git::is_repository(&root) {
        return Ok(report);
    }
    let cursor = store.candidates().get_meta(CURSOR)?;
    let commits = commits_in(&root, window, cursor.as_deref());
    if commits.is_empty() {
        return Ok(report);
    }
    let Some(mut blobs) = Blobs::open(&root) else {
        return Ok(report);
    };

    let mut kept = Vec::new();
    let mut changes = Vec::new();
    for commit in &commits {
        let changed = git::changed_files(&root, &commit.sha);
        if let Some(reason) = skip_reason(commit, changed.touched) {
            match reason {
                Skip::Bot => report.skipped_bots += 1,
                Skip::Revert => report.skipped_reverts += 1,
                Skip::Sweep => report.skipped_sweeps += 1,
            }
            continue;
        }
        let index = kept.len();
        kept.push(commit);
        let parent = format!("{}^", commit.sha);
        for (path, added) in changed.changes {
            if !SupportedLanguage::from_path(&path).is_some_and(SupportedLanguage::has_analyzers) {
                continue;
            }
            let before = if added {
                Some(String::new())
            } else {
                blobs.read(&parent, &path).and_then(text)
            };
            let after = blobs.read(&commit.sha, &path).and_then(text);
            let (Some(before), Some(after)) = (before, after) else {
                continue;
            };
            let fingerprint = change_fingerprint(&path, &before, &after);
            if store.candidates().is_learned(&fingerprint)? {
                report.already_learned += 1;
                continue;
            }
            changes.push(FileChange {
                commit: index,
                path,
                before,
                after,
                fingerprint,
            });
        }
    }
    drop(blobs);

    // The detectors are pure, so they run in parallel; their results are
    // recorded in commit order below.
    let paths = store.paths();
    let analysed: Vec<Vec<Observation>> = changes
        .par_iter()
        .map(|change| {
            taste_core::analyze_in(paths, &change.before, &change.after, &change.path)
                .unwrap_or_default()
        })
        .collect();

    let total = kept.len();
    let mut next = 0;
    for (index, commit) in kept.iter().enumerate() {
        while let Some(change) = changes.get(next).filter(|change| change.commit == index) {
            // Before the observations, so an interrupted run never counts a
            // change twice; at worst it misses this one.
            store.candidates().mark_learned(&change.fingerprint)?;
            for observation in analysed[next].iter().cloned() {
                if store
                    .candidates()
                    .take_delta(&delta_key(&change.path, &observation))?
                {
                    report.already_learned += 1;
                    continue;
                }
                report.outcomes.push(store.record_observation(observation)?);
            }
            next += 1;
        }
        store.candidates().set_meta(CURSOR, &commit.sha)?;
        report.commits += 1;
        progress(index + 1, total);
    }
    // The last commit in the window is the newest, skipped or not: the next
    // run starts after it.
    if let Some(newest) = commits.last() {
        store.candidates().set_meta(CURSOR, &newest.sha)?;
    }
    let read = store
        .candidates()
        .get_meta(COMMITS_READ)?
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    store
        .candidates()
        .set_meta(COMMITS_READ, &(read + report.commits).to_string())?;
    store
        .candidates()
        .set_meta(READ_AT, &Utc::now().to_rfc3339())?;
    Ok(report)
}

fn text(bytes: Vec<u8>) -> Option<String> {
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// How much history has been read, for `taste status`: commits learned from,
/// the newest commit read, and when.
pub fn summary(store: &RuleStore) -> anyhow::Result<Option<(usize, String, String)>> {
    let meta = |key| store.candidates().get_meta(key);
    let (Some(commits), Some(cursor), Some(at)) =
        (meta(COMMITS_READ)?, meta(CURSOR)?, meta(READ_AT)?)
    else {
        return Ok(None);
    };
    Ok(Some((
        commits.parse().unwrap_or(0),
        cursor.chars().take(7).collect(),
        at,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(author: &str, email: &str, subject: &str, body: &str) -> Commit {
        Commit {
            sha: "abc".into(),
            author: author.into(),
            email: email.into(),
            subject: subject.into(),
            body: body.into(),
        }
    }

    #[test]
    fn bots_reverts_and_sweeps_are_skipped() {
        let human = commit("Ada", "ada@example.com", "Use the logger", "");
        assert_eq!(skip_reason(&human, 3), None);
        assert_eq!(skip_reason(&human, MAX_FILES), None);
        assert_eq!(skip_reason(&human, MAX_FILES + 1), Some(Skip::Sweep));
        for bot in [
            commit(
                "dependabot[bot]",
                "49699333+dependabot[bot]@users.noreply.github.com",
                "Bump x",
                "",
            ),
            commit("renovate[bot]", "bot@renovateapp.com", "Update y", ""),
            commit(
                "github-actions",
                "41898282+github-actions[bot]@users.noreply.github.com",
                "Release",
                "",
            ),
        ] {
            assert_eq!(skip_reason(&bot, 1), Some(Skip::Bot), "{}", bot.author);
        }
        let revert = commit("Ada", "ada@example.com", "Revert \"Use the logger\"", "");
        assert_eq!(skip_reason(&revert, 1), Some(Skip::Revert));
        let quiet = commit(
            "Ada",
            "ada@example.com",
            "Undo",
            "This reverts commit 1234567.",
        );
        assert_eq!(skip_reason(&quiet, 1), Some(Skip::Revert));
    }
}
