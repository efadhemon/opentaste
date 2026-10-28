//! The candidate cache is disposable by design, but it still has to survive
//! being closed, reopened and shared between processes without losing data or
//! re-running its migrations.

mod common;

use chrono::Utc;
use taste_core::models::{CandidateRule, RuleCategory, RuleSource};
use taste_core::storage::{FileSnapshot, SqliteStore};

fn candidate(instruction: &str) -> CandidateRule {
    CandidateRule::new(
        RuleCategory::Architecture,
        "src/**/*.entity.ts",
        RuleSource::AstDiff,
        instruction,
    )
}

#[test]
fn the_cache_file_and_its_directory_are_created_on_demand() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(".taste").join("cache.db");
    assert!(!path.exists());

    let store = SqliteStore::open(&path).unwrap();
    assert!(path.exists());
    assert_eq!(store.count_candidates().unwrap(), 0);
}

#[test]
fn data_and_schema_version_survive_repeated_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.db");

    let stored = {
        let store = SqliteStore::open(&path).unwrap();
        let c = candidate("Use @AutoEntity rather than the raw decorator");
        store.upsert_candidate(&c).unwrap();
        c
    };

    for _ in 0..3 {
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.count_candidates().unwrap(), 1);
        assert_eq!(
            store.get_candidate(&stored.id).unwrap().as_ref(),
            Some(&stored)
        );
    }
}

#[test]
fn two_open_handles_see_each_others_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.db");

    let writer = SqliteStore::open(&path).unwrap();
    let reader = SqliteStore::open(&path).unwrap();

    let c = candidate("Use @AutoEntity rather than the raw decorator");
    writer.upsert_candidate(&c).unwrap();

    assert_eq!(reader.get_candidate(&c.id).unwrap(), Some(c.clone()));

    writer.delete_candidate(&c.id).unwrap();
    assert_eq!(reader.get_candidate(&c.id).unwrap(), None);
}

#[test]
fn snapshots_let_a_rescan_skip_unchanged_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.db");
    let store = SqliteStore::open(&path).unwrap();

    let snapshot = FileSnapshot {
        file_path: "src/users/user.entity.ts".to_string(),
        content_hash: "sha256:abc".to_string(),
        ast_signature: "class:User;decorator:AutoEntity".to_string(),
        captured_at: Utc::now(),
    };
    store.upsert_snapshot(&snapshot).unwrap();

    let reopened = SqliteStore::open(&path).unwrap();
    let loaded = reopened.get_snapshot(&snapshot.file_path).unwrap().unwrap();
    assert_eq!(loaded.content_hash, snapshot.content_hash);
    assert_eq!(loaded.ast_signature, snapshot.ast_signature);
    assert_eq!(
        loaded.captured_at.timestamp_micros(),
        snapshot.captured_at.timestamp_micros(),
        "timestamps must survive the RFC 3339 round trip"
    );
}

#[test]
fn candidates_keep_their_confidence_to_two_decimals_across_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteStore::open(dir.path().join("cache.db")).unwrap();

    let mut c = candidate("Use @AutoEntity rather than the raw decorator");
    for confidence in [0.45_f32, 0.56, 0.65, 0.72, 0.78, 0.82] {
        c.confidence = confidence;
        store.upsert_candidate(&c).unwrap();
        assert_eq!(
            store.get_candidate(&c.id).unwrap().unwrap().confidence,
            confidence,
            "confidence must not drift through SQLite's REAL column"
        );
    }
}
