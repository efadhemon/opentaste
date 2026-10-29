//! Parallel repository walk.
//!
//! Paths are collected with the serial `ignore` walker and then parsed on the
//! rayon pool. `ignore`'s parallel walker has its own thread pool and a
//! callback API, which does not compose with rayon's `fold`/`reduce` without a
//! lock around the tally. The walk is not the bottleneck. Snapshots and
//! observations are written only after the parallel phase, and observations
//! are recorded before snapshots: a crash in between re-scans the files
//! instead of permanently skipping a promotion that never landed.

use std::fs;
use std::path::Path;

use chrono::Utc;
use ignore::WalkBuilder;
use rayon::prelude::*;
use serde_json::Value;

use crate::error::Result;
use crate::models::ProjectContext;
use crate::storage::{FileSnapshot, Outcome, RuleStore};

use crate::models::rule_id;

use super::analyzers::{decode_tally, encode_tally};
use super::parser::content_hash;
use super::promote;
use super::{analyze_source, SupportedLanguage, Tally};

/// Files at or above this size are skipped. A megabyte is far past a source
/// file the statistical detectors can usefully read.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// What one cold-start scan did.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanReport {
    /// Files parsed on this run.
    pub files_scanned: usize,
    /// Supported files that were not parsed: unchanged, too large, or not UTF-8.
    pub files_skipped: usize,
    /// Counts for the whole repository.
    ///
    /// Unchanged files contribute the tally stored in their snapshot, so a
    /// rescan does not recompute the ratio from only the files that moved.
    pub tallies: Tally,
    /// Outcomes of the observations this scan recorded.
    pub outcomes: Vec<Outcome>,
}

struct PendingFile {
    relative: String,
    hash: String,
    source: String,
    lang: SupportedLanguage,
}

/// Scan the project owned by `store` and record any rule the tally supports.
pub fn scan(store: &RuleStore) -> Result<ScanReport> {
    let root = &store.paths().project_root;
    let mut pending = Vec::new();
    let mut cached: Vec<Tally> = Vec::new();
    let mut files_skipped = 0usize;
    let mut unchanged = 0usize;

    for entry in WalkBuilder::new(root)
        .standard_filters(true)
        // A `.gitignore` is the developer's exclusion list even before `git init`.
        .require_git(false)
        .git_global(false)
        // The root itself is never filtered: a project checked out as `build/`
        // is still a project.
        .filter_entry(|entry| entry.depth() == 0 || !is_generated(entry.path(), entry.file_type()))
        .build()
    {
        let Ok(entry) = entry else {
            continue;
        };
        let path = entry.path();
        let Some(lang) = SupportedLanguage::from_path(path) else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            files_skipped += 1;
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        if metadata.len() > MAX_FILE_BYTES {
            files_skipped += 1;
            continue;
        }
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => {
                files_skipped += 1;
                continue;
            }
        };
        let Ok(source) = String::from_utf8(bytes) else {
            files_skipped += 1;
            continue;
        };
        let hash = content_hash(source.as_bytes());
        let relative = relative_key(root, path);
        if let Some(snapshot) = store.candidates().get_snapshot(&relative)? {
            if snapshot.content_hash == hash {
                if let Some(tally) = decode_tally(&snapshot.ast_signature) {
                    cached.push(tally);
                    unchanged += 1;
                    files_skipped += 1;
                    continue;
                }
            }
        }
        pending.push(PendingFile {
            relative,
            hash,
            source,
            lang,
        });
    }

    let fresh: Vec<Tally> = pending
        .par_iter()
        .map(|file| analyze_source(file.lang, &file.source))
        .collect();
    let mut tallies = cached.into_iter().fold(Tally::default(), |mut acc, item| {
        acc += item;
        acc
    });
    for tally in &fresh {
        tallies += *tally;
    }

    let stack = detect_stack(root);
    let files_scanned = pending.len();
    store.set_project_context(ProjectContext {
        detected_stack: stack,
        total_files_scanned: files_scanned + unchanged,
        last_updated: Utc::now(),
    })?;

    let mut outcomes = Vec::new();
    for observation in promote::observations(&tallies) {
        if already_recorded(store, &observation)? {
            continue;
        }
        outcomes.push(store.record_observation(observation)?);
    }

    let captured_at = Utc::now();
    for (file, tally) in pending.iter().zip(&fresh) {
        store.candidates().upsert_snapshot(&FileSnapshot {
            file_path: file.relative.clone(),
            content_hash: file.hash.clone(),
            ast_signature: encode_tally(tally),
            captured_at,
        })?;
    }

    Ok(ScanReport {
        files_scanned,
        files_skipped,
        tallies,
        outcomes,
    })
}

/// Directories that hold installed, built or vendored code, skipped whether
/// or not a `.gitignore` lists them. Somebody else's code is not this
/// project's taste, and missing a hand-written file under `build/` costs far
/// less than a ratio skewed by a bundle.
const GENERATED_DIRS: &[&str] = &[
    "node_modules",
    "bower_components",
    "vendor",
    "dist",
    "build",
    "out",
    "coverage",
    "target",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
];

/// File suffixes that are declarations or build output rather than code a
/// developer wrote. `.d.ts` files are nearly all `export interface`, which
/// would skew both the export and the type ratio.
const GENERATED_SUFFIXES: &[&str] = &[
    ".d.ts",
    ".d.mts",
    ".d.cts",
    ".min.js",
    ".min.mjs",
    ".bundle.js",
];

fn is_generated(path: &Path, file_type: Option<std::fs::FileType>) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if file_type.is_some_and(|kind| kind.is_dir()) {
        return GENERATED_DIRS.contains(&name);
    }
    let lower = name.to_ascii_lowercase();
    GENERATED_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn already_recorded(store: &RuleStore, observation: &crate::storage::Observation) -> Result<bool> {
    let id = rule_id(observation.category, &observation.instruction);
    // A deleted rule is the developer's answer to this exact ratio.
    if store.candidates().is_rejected(&id)? {
        return Ok(true);
    }
    if store.rules()?.find(&id).is_some() {
        return Ok(true);
    }
    if let Some(opposite) = promote::opposite_instruction(&observation.instruction) {
        let opposite_id = rule_id(observation.category, opposite);
        if store.rules()?.find(&opposite_id).is_some() {
            return Ok(true);
        }
    }
    Ok(store.candidates().get_candidate(&id)?.is_some())
}

fn relative_key(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn detect_stack(root: &Path) -> Vec<String> {
    let mut stack = Vec::new();
    if root.join("Cargo.toml").is_file() {
        stack.push("rust".to_string());
    }
    if root.join("pyproject.toml").is_file() {
        stack.push("python".to_string());
    }
    if root.join("go.mod").is_file() {
        stack.push("go".to_string());
    }
    if let Some(names) = package_dependency_names(root) {
        for name in names {
            if let Some(label) = stack_label(&name) {
                if !stack.iter().any(|existing| existing == label) {
                    stack.push(label.to_string());
                }
            }
        }
    }
    stack.sort();
    stack
}

fn package_dependency_names(root: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(root.join("package.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let mut names = Vec::new();
    for key in ["dependencies", "devDependencies", "peerDependencies"] {
        if let Some(table) = value.get(key).and_then(Value::as_object) {
            names.extend(table.keys().cloned());
        }
    }
    Some(names)
}

fn stack_label(name: &str) -> Option<&'static str> {
    match name {
        "react" | "react-dom" => Some("react"),
        "next" => Some("next"),
        "vue" => Some("vue"),
        "express" => Some("express"),
        "svelte" => Some("svelte"),
        other if other.starts_with("@nestjs/") => Some("nestjs"),
        other if other.starts_with("@angular/") => Some("angular"),
        other if other.starts_with("@sveltejs/") => Some("svelte"),
        _ => None,
    }
}
