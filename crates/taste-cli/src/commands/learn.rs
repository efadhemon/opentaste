//! `taste learn --from-git-head`: what the post-commit hook runs.
//!
//! Diffs `HEAD` against its parent (or against nothing, for the first
//! commit), learns from each changed script file, and decays any rule whose
//! anti-pattern the commit added. Walking further back through history
//! is `taste scan --history`'s job ([`crate::commands::history`]), which shares
//! this command's fingerprints so the two never count one change twice.
//!
//! Each file's change is learned from once. The hook fires again on `git
//! commit --amend`, a cherry-pick or a rebuilt branch, and the command can be
//! run by hand; a change already in the cache is skipped, or one correction
//! would be counted once per run.
//!
//! A correction the editor extension already learned through `learn-delta`
//! is skipped too, once: the extension saw it when the file was saved, and
//! the commit carries the same edit.

use std::collections::BTreeSet;
use std::path::Path;

use taste_core::{contradictions, RuleStore, SupportedLanguage};

use crate::commands::{change_fingerprint, delta_key, describe_outcome, sync};
use crate::git;
use crate::output::Output;

/// Larger files are skipped, as the scanner skips them.
const MAX_FILE_BYTES: usize = 1024 * 1024;

pub fn run(store: &RuleStore, out: &Output) -> anyhow::Result<()> {
    let root = store.paths().project_root.clone();
    if git::text(&root, &["rev-parse", "--verify", "-q", "HEAD"]).is_none() {
        anyhow::bail!("no commit to learn from: {} has no HEAD", root.display());
    }
    let has_parent = git::text(&root, &["rev-parse", "--verify", "-q", "HEAD~1"]).is_some();

    let mut learned = 0usize;
    let mut promoted = false;
    let mut contradicted = BTreeSet::new();
    let mut fingerprints = Vec::new();
    let mut seen_before = 0usize;
    let mut from_editor = 0usize;
    for (path, added) in git::changed_files(&root, "HEAD").changes {
        if !SupportedLanguage::from_path(&path).is_some_and(SupportedLanguage::has_analyzers) {
            continue;
        }
        let before = if added || !has_parent {
            Some(String::new())
        } else {
            show(&root, "HEAD~1", &path)
        };
        let (Some(before), Some(after)) = (before, show(&root, "HEAD", &path)) else {
            continue;
        };
        let fingerprint = change_fingerprint(&path, &before, &after);
        if store.candidates().is_learned(&fingerprint)? {
            seen_before += 1;
            continue;
        }
        fingerprints.push(fingerprint);
        for observation in taste_core::analyze_in(store.paths(), &before, &after, &path)? {
            if store
                .candidates()
                .take_delta(&delta_key(&path, &observation))?
            {
                from_editor += 1;
                out.say(format!(
                    "  already    learned from the editor: {}",
                    observation.instruction
                ));
                continue;
            }
            let outcome = store.record_observation(observation)?;
            promoted |= outcome.promoted().is_some();
            learned += 1;
            out.say(format!("  {}", describe_outcome(&outcome)));
        }
        contradicted.extend(contradictions(store, &path, &before, &after)?);
    }
    for id in &contradicted {
        if store.apply_contradiction(id)? {
            out.say(format!(
                "  decayed    {id}: this commit added its anti-pattern"
            ));
        }
    }
    // Marked only once everything above succeeded, so a failed run is retried.
    for fingerprint in &fingerprints {
        store.candidates().mark_learned(fingerprint)?;
    }
    if learned == 0 && from_editor == 0 && contradicted.is_empty() {
        if seen_before > 0 && fingerprints.is_empty() {
            out.say("Already learned from the changes in HEAD.");
        } else {
            out.say("Nothing to learn from HEAD.");
        }
    }
    if promoted || !contradicted.is_empty() {
        let report = store.sync()?;
        sync::report_sync(&report, store, out);
    }
    Ok(())
}

/// The contents of `path` at `revision`, when it is UTF-8 and not huge.
fn show(root: &Path, revision: &str, path: &Path) -> Option<String> {
    let spec = format!("{revision}:./{}", path.to_string_lossy().replace('\\', "/"));
    let bytes = git::output(root, &["show", &spec])?;
    if bytes.len() > MAX_FILE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}
