//! Parallel repository walk.
//!
//! Files are found, read and hashed on the `ignore` crate's parallel walker,
//! collected under one lock, and sorted by path so the result does not depend
//! on which walker thread finished first. Every cached snapshot is loaded
//! once into a map. Changed files are then parsed and counted on the rayon
//! pool. Snapshots and observations are written only after the parallel
//! phase, snapshots in one transaction, and observations are recorded before
//! snapshots: a crash in between re-scans the files instead of permanently
//! skipping a promotion that never landed.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use globset::{GlobBuilder, GlobSetBuilder};
use ignore::{DirEntry, WalkBuilder, WalkState};
use rayon::prelude::*;
use serde_json::Value;

use crate::error::Result;
use crate::models::{ProjectContext, RuleSource, TasteRule};
use crate::storage::{FileSnapshot, Outcome, RuleStore};

use crate::models::rule_id;

use super::analyzers::{decode_tally, encode_tally};
use super::parser::content_hash;
use super::promote;
use super::tsconfig;
use super::{analyze_source, is_test_or_script, SupportedLanguage, Tally};

/// Files larger than this are skipped. 256 KiB is several thousand lines,
/// past nearly every file a person writes by hand, and what lies beyond it is
/// mostly generated or vendored: a 13,000-line `datatables.js` copied into
/// `src/static/` outweighed every script the project wrote itself.
const MAX_FILE_BYTES: u64 = 256 * 1024;

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
    /// Rules read from configuration whose setting is gone, removed by this
    /// scan. A ratio never retracts a rule; a compiler setting is a fact, so
    /// when it changes the rule goes with it.
    pub retracted: Vec<TasteRule>,
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
    let mut unchanged = 0usize;
    let mut seen = HashSet::new();

    let walked = walk(root);
    let mut files_skipped = walked.skipped;
    let tsconfigs = walked.tsconfigs;
    // One query for every snapshot rather than one per file.
    let snapshots: HashMap<String, FileSnapshot> = store
        .candidates()
        .list_snapshots()?
        .into_iter()
        .map(|snapshot| (snapshot.file_path.clone(), snapshot))
        .collect();
    for file in walked.files {
        seen.insert(file.relative.clone());
        let reused = snapshots
            .get(&file.relative)
            .filter(|snapshot| snapshot.content_hash == file.hash)
            .and_then(|snapshot| decode_tally(&snapshot.ast_signature));
        if let Some(tally) = reused {
            cached.push(tally);
            unchanged += 1;
            files_skipped += 1;
            continue;
        }
        pending.push(file);
    }

    let fresh: Vec<Tally> = pending
        .par_iter()
        .map(|file| analyze_source(file.lang, Path::new(&file.relative), &file.source))
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

    let strict_scopes = tsconfig::strict_scopes(root, &tsconfigs);
    let mut retracted = Vec::new();
    for rule in store.rules()?.rules {
        let stale = rule.source == RuleSource::AstScan
            && rule.instruction == promote::TS_STRICT_NULL
            && !strict_scopes.contains(&rule.scope);
        if stale && store.retract_rule(&rule.id)? {
            retracted.push(rule);
        }
    }
    let mut outcomes = Vec::new();
    let found = promote::observations(&tallies)
        .into_iter()
        .chain(promote::strict_null_observations(&strict_scopes));
    for observation in found {
        if already_recorded(store, &observation)? {
            continue;
        }
        outcomes.push(store.record_observation(observation)?);
    }

    let captured_at = Utc::now();
    let written: Vec<FileSnapshot> = pending
        .iter()
        .zip(&fresh)
        .map(|(file, tally)| FileSnapshot {
            file_path: file.relative.clone(),
            content_hash: file.hash.clone(),
            ast_signature: encode_tally(tally),
            captured_at,
        })
        .collect();
    store.candidates().upsert_snapshots(&written)?;
    store.candidates().retain_snapshots(&seen)?;

    Ok(ScanReport {
        files_scanned,
        files_skipped,
        tallies,
        outcomes,
        retracted,
    })
}

/// What the walk found, sorted by path so a scan is deterministic whatever
/// order the walker threads finished in.
struct Walked {
    files: Vec<PendingFile>,
    tsconfigs: Vec<PathBuf>,
    /// Supported files not read: too large, unreadable or not UTF-8.
    skipped: usize,
}

enum Found {
    File(PendingFile),
    Tsconfig(PathBuf),
    Skipped,
}

/// Walk `root` on the `ignore` crate's parallel walker, reading and hashing
/// each source file in the walker thread that found it.
fn walk(root: &Path) -> Walked {
    let found = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .standard_filters(true)
        // A `.gitignore` is the developer's exclusion list even before `git init`.
        .require_git(false)
        .git_global(false)
        // The root itself is never filtered: a project checked out as `build/`
        // is still a project.
        .filter_entry(|entry| entry.depth() == 0 || !is_generated(entry.path(), entry.file_type()))
        .build_parallel()
        .run(|| {
            let found = &found;
            Box::new(move |entry| {
                if let Some(item) = entry.ok().and_then(|entry| visit(root, &entry)) {
                    found
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .push(item);
                }
                WalkState::Continue
            })
        });
    let mut walked = Walked {
        files: Vec::new(),
        tsconfigs: Vec::new(),
        skipped: 0,
    };
    for item in found
        .into_inner()
        .unwrap_or_else(|poison| poison.into_inner())
    {
        match item {
            Found::File(file) => walked.files.push(file),
            Found::Tsconfig(path) => walked.tsconfigs.push(path),
            Found::Skipped => walked.skipped += 1,
        }
    }
    walked.files.sort_by(|a, b| a.relative.cmp(&b.relative));
    walked.tsconfigs.sort();
    walked
}

/// Read and hash one walked entry, or say why it is not a file to scan.
fn visit(root: &Path, entry: &DirEntry) -> Option<Found> {
    let path = entry.path();
    if path
        .file_name()
        .is_some_and(|name| name == tsconfig::TSCONFIG)
    {
        return Some(Found::Tsconfig(PathBuf::from(relative_key(root, path))));
    }
    // Checked before anything is read: a language with no analyzers would
    // be read, hashed, parsed and snapshotted for no counts at all.
    let lang = SupportedLanguage::from_path(path).filter(|lang| lang.has_analyzers())?;
    let relative = relative_key(root, path);
    if is_test_or_script(Path::new(&relative), lang) {
        return None;
    }
    let Ok(metadata) = entry.metadata() else {
        return Some(Found::Skipped);
    };
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Some(Found::Skipped);
    }
    let Some(source) = fs::read(path)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return Some(Found::Skipped);
    };
    let hash = content_hash(source.as_bytes());
    Some(Found::File(PendingFile {
        relative,
        hash,
        source,
        lang,
    }))
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
        || is_versioned(&lower)
}

/// A file named after a release, such as `jquery-3.7.1.js` or
/// `jdenticon-3.3.0.min.js`: a copy of somebody else's library. The version
/// needs at least two dotted parts, so `schema.v2.ts` or `0001_initial.py`
/// are not caught.
fn is_versioned(name: &str) -> bool {
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem = stem.strip_suffix(".min").unwrap_or(stem);
    let Some(split) = stem.rfind(['-', '_', '@']) else {
        return false;
    };
    let version = stem[split + 1..].trim_start_matches('v');
    let parts: Vec<_> = version.split('.').collect();
    parts.len() >= 2
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// The repository tally as of the last scan, read back from the snapshot
/// cache without parsing anything. Files deleted since that scan are left out.
pub fn stored_tally(store: &RuleStore) -> Result<Tally> {
    let root = &store.paths().project_root;
    let mut tally = Tally::default();
    for snapshot in store.candidates().list_snapshots()? {
        if !root.join(&snapshot.file_path).is_file() {
            continue;
        }
        if let Some(file) = decode_tally(&snapshot.ast_signature) {
            tally += file;
        }
    }
    Ok(tally)
}

fn already_recorded(store: &RuleStore, observation: &crate::storage::Observation) -> Result<bool> {
    let id = rule_id(
        observation.category,
        &observation.scope,
        &observation.instruction,
    );
    // A deleted rule is the developer's answer to this exact ratio.
    if store.candidates().is_rejected(&id)? {
        return Ok(true);
    }
    if store.rules()?.find(&id).is_some() {
        return Ok(true);
    }
    for opposite in promote::conflicting_instructions(&observation.instruction) {
        let opposite_id = rule_id(observation.category, &observation.scope, opposite);
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

/// Languages, frameworks and databases the project uses, read from the root
/// package and every workspace package of a monorepo.
fn detect_stack(root: &Path) -> Vec<String> {
    let mut stack = BTreeSet::new();
    let mut packages = vec![root.to_path_buf()];
    packages.extend(workspace_packages(root));
    for dir in &packages {
        for (marker, label) in [
            ("Cargo.toml", "rust"),
            ("pyproject.toml", "python"),
            ("go.mod", "go"),
            ("tsconfig.json", "typescript"),
        ] {
            if dir.join(marker).is_file() {
                stack.insert(label);
            }
        }
        for name in package_dependency_names(dir).unwrap_or_default() {
            if let Some(label) = stack_label(&name) {
                stack.insert(label);
            }
        }
    }
    stack.into_iter().map(str::to_string).collect()
}

/// Directories of a JavaScript monorepo's workspace packages, from
/// `workspaces` in the root `package.json` and from `pnpm-workspace.yaml`.
pub(crate) fn workspace_packages(root: &Path) -> Vec<PathBuf> {
    let mut patterns = Vec::new();
    if let Ok(text) = fs::read_to_string(root.join("package.json")) {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            let list = value
                .get("workspaces")
                .and_then(|w| w.as_array().or_else(|| w.get("packages")?.as_array()));
            patterns.extend(
                list.into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string),
            );
        }
    }
    if let Ok(text) = fs::read_to_string(root.join("pnpm-workspace.yaml")) {
        patterns.extend(pnpm_workspace_patterns(&text));
    }
    if patterns.is_empty() {
        return Vec::new();
    }

    let set = |negated: bool| {
        let mut builder = GlobSetBuilder::new();
        for pattern in &patterns {
            let (is_negated, body) = match pattern.strip_prefix('!') {
                Some(body) => (true, body),
                None => (false, pattern.as_str()),
            };
            if is_negated == negated {
                if let Ok(glob) = GlobBuilder::new(body.trim_end_matches('/'))
                    .literal_separator(true)
                    .build()
                {
                    builder.add(glob);
                }
            }
        }
        builder.build().ok()
    };
    let (Some(include), Some(exclude)) = (set(false), set(true)) else {
        return Vec::new();
    };

    WalkBuilder::new(root)
        .max_depth(Some(WORKSPACE_DEPTH))
        .filter_entry(|entry| entry.file_name() != "node_modules")
        .build()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_some_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let relative = entry.path().strip_prefix(root).ok()?.to_path_buf();
            let matched = !relative.as_os_str().is_empty()
                && include.is_match(&relative)
                && !exclude.is_match(&relative)
                && entry.path().join("package.json").is_file();
            matched.then(|| entry.path().to_path_buf())
        })
        .collect()
}

/// The `packages:` list of a `pnpm-workspace.yaml`. Only that list is read,
/// which keeps a YAML parser out of the dependency tree.
fn pnpm_workspace_patterns(text: &str) -> Vec<String> {
    let mut patterns = Vec::new();
    let mut in_packages = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !line.starts_with([' ', '\t', '-']) {
            in_packages = trimmed == "packages:";
            continue;
        }
        if let (true, Some(item)) = (in_packages, trimmed.strip_prefix('-')) {
            let item = item.split(" #").next().unwrap_or(item).trim();
            patterns.push(item.trim_matches(['\'', '"']).to_string());
        }
    }
    patterns
}

/// How deep workspace packages are looked for: `apps/*` and `packages/*/*`.
const WORKSPACE_DEPTH: usize = 4;

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
        "typescript" => Some("typescript"),
        "pg" | "postgres" | "pg-promise" => Some("postgresql"),
        "mysql" | "mysql2" => Some("mysql"),
        "mongodb" | "mongoose" => Some("mongodb"),
        "sqlite3" | "better-sqlite3" => Some("sqlite"),
        "prisma" | "@prisma/client" => Some("prisma"),
        "typeorm" => Some("typeorm"),
        "drizzle-orm" => Some("drizzle"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_named_after_a_release_is_vendored() {
        for name in [
            "jquery-3.7.1.js",
            "jdenticon-3.3.0.js",
            "lib_v1.2.min.js",
            "vue@3.4.0.mjs",
        ] {
            assert!(is_versioned(name), "{name}");
        }
        for name in [
            "schema.v2.ts",
            "0001_initial.py",
            "app-2.ts",
            "admin.js",
            "v1.rs",
            "a-.js",
        ] {
            assert!(!is_versioned(name), "{name}");
        }
    }
}
