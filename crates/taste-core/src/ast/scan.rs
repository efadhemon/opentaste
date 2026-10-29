//! Parallel repository walk.
//!
//! Paths are collected with the serial `ignore` walker and then parsed on the
//! rayon pool. `ignore`'s parallel walker has its own thread pool and a
//! callback API, which does not compose with rayon's `fold`/`reduce` without a
//! lock around the tally. The walk is not the bottleneck. Snapshots and
//! observations are written only after the parallel phase, and observations
//! are recorded before snapshots: a crash in between re-scans the files
//! instead of permanently skipping a promotion that never landed.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use globset::{GlobBuilder, GlobSetBuilder};
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
use super::{analyze_source, is_test_or_script, SupportedLanguage, Tally};

/// Files larger than this are skipped. A megabyte is far past a source file
/// the statistical detectors can usefully read.
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
    let mut seen = HashSet::new();

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
        // Checked before anything is read: a language with no analyzers would
        // be read, hashed, parsed and snapshotted for no counts at all.
        let Some(lang) = SupportedLanguage::from_path(path).filter(|lang| lang.has_analyzers())
        else {
            continue;
        };
        if is_test_or_script(Path::new(&relative_key(root, path)), lang) {
            continue;
        }
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
        seen.insert(relative.clone());
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
    store.candidates().retain_snapshots(&seen)?;

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
    if let Some(opposite) = promote::opposite_instruction(&observation.instruction) {
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
fn workspace_packages(root: &Path) -> Vec<PathBuf> {
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
