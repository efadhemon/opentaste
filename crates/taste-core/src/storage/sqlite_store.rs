//! The local candidate cache, `.taste/cache.db`.
//!
//! Everything in here is deliberately *not* committed: patterns that have not
//! yet earned a developer's trust, plus the file hashes the scanner uses to
//! skip unchanged files. Keeping them out of git is what lets OpenTaste observe
//! aggressively without generating review noise.

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::error::{Error, Result};
use crate::models::{round2, CandidateRule, RuleCategory, RuleSource};
use crate::storage::atomic::ensure_parent_dir;

/// Schema version this build expects; bumped whenever a migration is added.
const SCHEMA_VERSION: i32 = 5;

/// A recorded AST fingerprint for one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    pub file_path: String,
    pub content_hash: String,
    pub ast_signature: String,
    pub captured_at: DateTime<Utc>,
}

/// Handle to a candidate database, project-local or global.
#[derive(Debug)]
pub struct SqliteStore {
    conn: Connection,
}

impl SqliteStore {
    /// Open (creating if needed) a database at `path` and bring it up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        ensure_parent_dir(path)?;
        let conn = Connection::open(path)?;
        Self::from_connection(conn)
    }

    /// Open a throwaway in-memory database, used by tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        // WAL lets the MCP server read while the CLI writes; `NORMAL` sync is
        // the right trade for a cache that can always be rebuilt by rescanning.
        // `journal_mode` reports the mode it settled on, so it has to be run as
        // a query; in-memory databases simply stay in `memory` mode.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;

        let store = SqliteStore { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Apply any migrations this database has not seen yet.
    fn migrate(&self) -> Result<()> {
        let current: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current >= SCHEMA_VERSION {
            return Ok(());
        }

        if current < 1 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS candidate_rules (
                    id TEXT PRIMARY KEY,
                    category TEXT NOT NULL,
                    scope TEXT NOT NULL,
                    confidence REAL NOT NULL DEFAULT 0.35,
                    occurrences INTEGER NOT NULL DEFAULT 1,
                    contradictions INTEGER NOT NULL DEFAULT 0,
                    source TEXT NOT NULL,
                    instruction TEXT NOT NULL,
                    anti_pattern TEXT,
                    preferred_pattern TEXT,
                    updated_at TEXT NOT NULL
                );

                CREATE INDEX IF NOT EXISTS idx_candidate_rules_category
                    ON candidate_rules (category);

                CREATE TABLE IF NOT EXISTS file_snapshots (
                    file_path TEXT PRIMARY KEY,
                    content_hash TEXT NOT NULL,
                    ast_signature TEXT NOT NULL,
                    captured_at TEXT NOT NULL
                );
                "#,
            )?;
        }

        if current < 2 {
            // A rule the developer deleted. The scanner recomputes the same
            // ratio on every run, so without this it would install the rule
            // again on the next scan.
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS rejected_rules (
                    id TEXT PRIMARY KEY,
                    rejected_at TEXT NOT NULL
                );
                "#,
            )?;
        }

        if current < 3 {
            // Changes `taste learn --from-git-head` has already learned from.
            // The post-commit hook fires again on `git commit --amend`, a
            // cherry-pick or a rebuilt branch, and the command can also be run
            // by hand; without this, one correction would count once per run.
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS learned_changes (
                    fingerprint TEXT PRIMARY KEY,
                    learned_at TEXT NOT NULL
                );
                "#,
            )?;
        }

        if current < 4 {
            // Observations the editor extension recorded through
            // `learn-delta`. When the corrected code is committed, the
            // post-commit hook finds the same correction in the commit; this
            // is how it knows not to count it a second time.
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS pending_deltas (
                    key TEXT PRIMARY KEY,
                    recorded_at TEXT NOT NULL
                );
                "#,
            )?;
        }

        if current < 5 {
            // Small facts about the cache itself, such as the newest commit
            // `taste scan --history` has read, so the next run starts there.
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS cache_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                "#,
            )?;
        }

        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    /// Insert a candidate or replace the existing one with the same identifier.
    pub fn upsert_candidate(&self, candidate: &CandidateRule) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO candidate_rules (
                id, category, scope, confidence, occurrences, contradictions,
                source, instruction, anti_pattern, preferred_pattern, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(id) DO UPDATE SET
                category = excluded.category,
                scope = excluded.scope,
                confidence = excluded.confidence,
                occurrences = excluded.occurrences,
                contradictions = excluded.contradictions,
                source = excluded.source,
                instruction = excluded.instruction,
                anti_pattern = excluded.anti_pattern,
                preferred_pattern = excluded.preferred_pattern,
                updated_at = excluded.updated_at
            "#,
            params![
                candidate.id,
                candidate.category.slug(),
                candidate.scope,
                candidate.confidence as f64,
                candidate.occurrences,
                candidate.contradictions,
                candidate.source.slug(),
                candidate.instruction,
                candidate.anti_pattern,
                candidate.preferred_pattern,
                candidate.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Fetch one candidate by identifier.
    pub fn get_candidate(&self, id: &str) -> Result<Option<CandidateRule>> {
        self.conn
            .query_row(
                &format!("{SELECT_CANDIDATE} WHERE id = ?1"),
                params![id],
                row_to_candidate,
            )
            .optional()?
            .transpose()
    }

    /// All candidates, ordered by identifier.
    pub fn list_candidates(&self) -> Result<Vec<CandidateRule>> {
        let mut stmt = self
            .conn
            .prepare(&format!("{SELECT_CANDIDATE} ORDER BY id"))?;
        let rows = stmt.query_map([], row_to_candidate)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// Candidates in one category, ordered by identifier.
    pub fn list_candidates_in(&self, category: RuleCategory) -> Result<Vec<CandidateRule>> {
        let mut stmt = self.conn.prepare(&format!(
            "{SELECT_CANDIDATE} WHERE category = ?1 ORDER BY id"
        ))?;
        let rows = stmt.query_map(params![category.slug()], row_to_candidate)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// Candidates that have cleared both promotion gates.
    pub fn list_promotable(&self) -> Result<Vec<CandidateRule>> {
        Ok(self
            .list_candidates()?
            .into_iter()
            .filter(crate::scorer::confidence::is_promotable)
            .collect())
    }

    /// Delete a candidate, reporting whether a row was removed.
    pub fn delete_candidate(&self, id: &str) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM candidate_rules WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    /// Remember that the developer deleted the rule with this identifier.
    pub fn reject(&self, id: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO rejected_rules (id, rejected_at) VALUES (?1, ?2)
            ON CONFLICT(id) DO UPDATE SET rejected_at = excluded.rejected_at
            "#,
            params![id, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Whether the developer deleted the rule with this identifier.
    pub fn is_rejected(&self, id: &str) -> Result<bool> {
        let found: Option<i32> = self
            .conn
            .query_row(
                "SELECT 1 FROM rejected_rules WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Whether the change with this fingerprint has already been learned from.
    pub fn is_learned(&self, fingerprint: &str) -> Result<bool> {
        let found: Option<i32> = self
            .conn
            .query_row(
                "SELECT 1 FROM learned_changes WHERE fingerprint = ?1",
                params![fingerprint],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// Remember that the change with this fingerprint has been learned from.
    pub fn mark_learned(&self, fingerprint: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO learned_changes (fingerprint, learned_at) VALUES (?1, ?2)",
            params![fingerprint, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// A value stored with [`SqliteStore::set_meta`].
    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM cache_meta WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Store `value` under `key`, replacing what was there.
    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO cache_meta (key, value) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
            params![key, value],
        )?;
        Ok(())
    }

    /// Remember an observation the editor extension just recorded.
    pub fn remember_delta(&self, key: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO pending_deltas (key, recorded_at) VALUES (?1, ?2)
            ON CONFLICT(key) DO UPDATE SET recorded_at = excluded.recorded_at
            "#,
            params![key, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    /// Forget an observation the editor extension recorded, returning whether
    /// it was there. Each one is taken at most once.
    pub fn take_delta(&self, key: &str) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM pending_deltas WHERE key = ?1", params![key])?;
        Ok(removed > 0)
    }

    /// Drop recorded observations older than `cutoff`: edits that were never
    /// committed, or were committed while the hook was not installed.
    pub fn prune_deltas(&self, cutoff: DateTime<Utc>) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM pending_deltas WHERE recorded_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        Ok(removed)
    }

    /// Number of stored candidates.
    pub fn count_candidates(&self) -> Result<u32> {
        let count: u32 =
            self.conn
                .query_row("SELECT COUNT(*) FROM candidate_rules", [], |row| row.get(0))?;
        Ok(count)
    }

    /// Record (or refresh) the fingerprint of a source file.
    pub fn upsert_snapshot(&self, snapshot: &FileSnapshot) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO file_snapshots (file_path, content_hash, ast_signature, captured_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(file_path) DO UPDATE SET
                content_hash = excluded.content_hash,
                ast_signature = excluded.ast_signature,
                captured_at = excluded.captured_at
            "#,
            params![
                snapshot.file_path,
                snapshot.content_hash,
                snapshot.ast_signature,
                snapshot.captured_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Fetch the fingerprint recorded for a source file.
    pub fn get_snapshot(&self, file_path: &str) -> Result<Option<FileSnapshot>> {
        self.conn
            .query_row(
                r#"
                SELECT file_path, content_hash, ast_signature, captured_at
                FROM file_snapshots WHERE file_path = ?1
                "#,
                params![file_path],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?
            .map(|(file_path, content_hash, ast_signature, captured_at)| {
                Ok(FileSnapshot {
                    file_path,
                    content_hash,
                    ast_signature,
                    captured_at: parse_timestamp(&captured_at)?,
                })
            })
            .transpose()
    }

    /// Delete the fingerprint of every file not in `keep`, returning how many
    /// went. The scanner passes the files its walk saw, so deleted, renamed
    /// and newly ignored files stop contributing stale tallies.
    pub fn retain_snapshots(&self, keep: &HashSet<String>) -> Result<usize> {
        let mut removed = 0;
        for snapshot in self.list_snapshots()? {
            if !keep.contains(&snapshot.file_path) {
                removed += self.conn.execute(
                    "DELETE FROM file_snapshots WHERE file_path = ?1",
                    params![snapshot.file_path],
                )?;
            }
        }
        Ok(removed)
    }

    /// Every recorded fingerprint, ordered by path.
    pub fn list_snapshots(&self) -> Result<Vec<FileSnapshot>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT file_path, content_hash, ast_signature, captured_at
            FROM file_snapshots ORDER BY file_path
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut snapshots = Vec::new();
        for row in rows {
            let (file_path, content_hash, ast_signature, captured_at) = row?;
            snapshots.push(FileSnapshot {
                file_path,
                content_hash,
                ast_signature,
                captured_at: parse_timestamp(&captured_at)?,
            });
        }
        Ok(snapshots)
    }

    /// Borrow the underlying connection, for callers needing custom queries.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

const SELECT_CANDIDATE: &str = r#"
    SELECT id, category, scope, confidence, occurrences, contradictions,
           source, instruction, anti_pattern, preferred_pattern, updated_at
    FROM candidate_rules
"#;

/// Map a row to a candidate.
///
/// The enum and timestamp columns are free-form text as far as SQLite is
/// concerned, so parsing them can fail on a hand-edited database. The failure
/// is carried out as an inner `Result` rather than being flattened into
/// `rusqlite::Error`, which keeps the richer error message intact.
#[allow(clippy::type_complexity)]
fn row_to_candidate(row: &Row<'_>) -> rusqlite::Result<Result<CandidateRule>> {
    let category: String = row.get(1)?;
    let source: String = row.get(6)?;
    let updated_at: String = row.get(10)?;
    let confidence: f64 = row.get(3)?;

    let id: String = row.get(0)?;
    let scope: String = row.get(2)?;
    let occurrences: u32 = row.get(4)?;
    let contradictions: u32 = row.get(5)?;
    let instruction: String = row.get(7)?;
    let anti_pattern: Option<String> = row.get(8)?;
    let preferred_pattern: Option<String> = row.get(9)?;

    Ok((|| {
        Ok(CandidateRule {
            id,
            category: category.parse::<RuleCategory>()?,
            scope,
            confidence: round2(confidence as f32),
            occurrences,
            contradictions,
            source: source.parse::<RuleSource>()?,
            instruction,
            anti_pattern,
            preferred_pattern,
            updated_at: parse_timestamp(&updated_at)?,
        })
    })())
}

fn parse_timestamp(raw: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| Error::InvalidEnumValue {
            kind: "RFC 3339 timestamp",
            value: raw.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RuleCategory;

    fn candidate(id_seed: &str) -> CandidateRule {
        CandidateRule::new(
            RuleCategory::Architecture,
            "src/**/*.entity.ts",
            RuleSource::AstDiff,
            format!("Use the {id_seed} decorator rather than the raw one"),
        )
    }

    #[test]
    fn a_version_one_database_gains_the_rejection_table_on_open() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE candidate_rules (id TEXT PRIMARY KEY);
                 CREATE TABLE file_snapshots (file_path TEXT PRIMARY KEY);
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        assert!(!store.is_rejected("imports-x").unwrap());
        store.reject("imports-x").unwrap();
        assert!(store.is_rejected("imports-x").unwrap());
    }

    #[test]
    fn a_version_two_database_gains_the_learned_changes_table_on_open() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cache.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE candidate_rules (id TEXT PRIMARY KEY);
                 CREATE TABLE file_snapshots (file_path TEXT PRIMARY KEY);
                 CREATE TABLE rejected_rules (id TEXT PRIMARY KEY, rejected_at TEXT NOT NULL);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
        }
        let store = SqliteStore::open(&path).unwrap();
        assert!(!store.is_learned("abc").unwrap());
        store.mark_learned("abc").unwrap();
        assert!(store.is_learned("abc").unwrap());
    }

    #[test]
    fn a_change_marked_learned_stays_learned_and_marking_it_again_is_harmless() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.mark_learned("abc").unwrap();
        store.mark_learned("abc").unwrap();
        assert!(store.is_learned("abc").unwrap());
        assert!(!store.is_learned("abd").unwrap());
    }

    #[test]
    fn a_remembered_delta_is_taken_once_and_old_ones_are_pruned() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.remember_delta("fresh").unwrap();
        store.remember_delta("stale").unwrap();
        store
            .connection()
            .execute(
                "UPDATE pending_deltas SET recorded_at = ?1 WHERE key = 'stale'",
                params![(Utc::now() - chrono::Duration::days(40)).to_rfc3339()],
            )
            .unwrap();

        assert_eq!(
            store
                .prune_deltas(Utc::now() - chrono::Duration::days(30))
                .unwrap(),
            1
        );
        assert!(!store.take_delta("stale").unwrap());
        assert!(store.take_delta("fresh").unwrap());
        assert!(!store.take_delta("fresh").unwrap(), "taken at most once");
    }

    #[test]
    fn a_fresh_database_has_both_tables_and_the_current_schema_version() {
        let store = SqliteStore::open_in_memory().unwrap();
        let version: i32 = store
            .connection()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        for table in [
            "candidate_rules",
            "file_snapshots",
            "rejected_rules",
            "learned_changes",
            "pending_deltas",
            "cache_meta",
        ] {
            let found: i32 = store
                .connection()
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(found, 1, "missing table {table}");
        }
    }

    #[test]
    fn a_meta_value_is_replaced_by_the_next_one() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.get_meta("history.cursor").unwrap(), None);
        store.set_meta("history.cursor", "abc").unwrap();
        store.set_meta("history.cursor", "def").unwrap();
        assert_eq!(
            store.get_meta("history.cursor").unwrap().as_deref(),
            Some("def")
        );
    }

    #[test]
    fn reopening_a_database_does_not_re_run_migrations_or_lose_data() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nested").join("cache.db");

        let stored = {
            let store = SqliteStore::open(&path).unwrap();
            let c = candidate("AutoEntity");
            store.upsert_candidate(&c).unwrap();
            c
        };

        let reopened = SqliteStore::open(&path).unwrap();
        assert_eq!(reopened.count_candidates().unwrap(), 1);
        assert_eq!(reopened.get_candidate(&stored.id).unwrap(), Some(stored));
    }

    #[test]
    fn upserting_the_same_id_updates_rather_than_duplicates() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut c = candidate("AutoEntity");
        store.upsert_candidate(&c).unwrap();

        c.occurrences = 7;
        c.confidence = 0.82;
        c.contradictions = 2;
        store.upsert_candidate(&c).unwrap();

        assert_eq!(store.count_candidates().unwrap(), 1);
        let loaded = store.get_candidate(&c.id).unwrap().unwrap();
        assert_eq!(loaded.occurrences, 7);
        assert_eq!(loaded.confidence, 0.82);
        assert_eq!(loaded.contradictions, 2);
    }

    #[test]
    fn candidates_round_trip_with_their_enums_patterns_and_timestamp() {
        let store = SqliteStore::open_in_memory().unwrap();
        let c = candidate("AutoIndex").with_patterns(
            Some("@Index()".to_string()),
            Some("@AutoIndex()".to_string()),
        );
        store.upsert_candidate(&c).unwrap();

        assert_eq!(store.get_candidate(&c.id).unwrap(), Some(c));
    }

    #[test]
    fn missing_candidates_are_reported_as_absent() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.get_candidate("nope").unwrap(), None);
        assert!(!store.delete_candidate("nope").unwrap());
    }

    #[test]
    fn listing_is_ordered_by_id_and_filterable_by_category() {
        let store = SqliteStore::open_in_memory().unwrap();
        store.upsert_candidate(&candidate("Zeta")).unwrap();
        store.upsert_candidate(&candidate("Alpha")).unwrap();

        let typed = CandidateRule::new(
            RuleCategory::Types,
            "src/**/*.ts",
            RuleSource::AstDiff,
            "Declare object shapes with interface rather than a type alias",
        );
        store.upsert_candidate(&typed).unwrap();

        let all = store.list_candidates().unwrap();
        assert_eq!(all.len(), 3);
        let mut sorted = all.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
        sorted.sort();
        assert_eq!(
            all.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            sorted,
            "list_candidates must be ordered by id"
        );

        let types_only = store.list_candidates_in(RuleCategory::Types).unwrap();
        assert_eq!(types_only, vec![typed]);
    }

    #[test]
    fn list_promotable_honours_both_promotion_gates() {
        let store = SqliteStore::open_in_memory().unwrap();

        let mut not_yet = candidate("Confident");
        not_yet.confidence = 0.85;
        not_yet.occurrences = 2; // high confidence, too few sightings
        store.upsert_candidate(&not_yet).unwrap();

        let mut unsure = candidate("Frequent");
        unsure.confidence = 0.60;
        unsure.occurrences = 9; // often seen, still uncertain
        store.upsert_candidate(&unsure).unwrap();

        let mut ready = candidate("Ready");
        ready.confidence = 0.82;
        ready.occurrences = 3;
        store.upsert_candidate(&ready).unwrap();

        let promotable = store.list_promotable().unwrap();
        assert_eq!(
            promotable.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
            vec![ready.id]
        );
    }

    #[test]
    fn file_snapshots_round_trip_and_update_in_place() {
        let store = SqliteStore::open_in_memory().unwrap();
        let mut snapshot = FileSnapshot {
            file_path: "src/users/user.entity.ts".to_string(),
            content_hash: "abc123".to_string(),
            ast_signature: "class:User;decorator:AutoEntity".to_string(),
            captured_at: DateTime::parse_from_rfc3339("2026-10-29T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        store.upsert_snapshot(&snapshot).unwrap();
        assert_eq!(
            store.get_snapshot(&snapshot.file_path).unwrap(),
            Some(snapshot.clone())
        );

        snapshot.content_hash = "def456".to_string();
        store.upsert_snapshot(&snapshot).unwrap();
        assert_eq!(
            store.get_snapshot(&snapshot.file_path).unwrap(),
            Some(snapshot)
        );

        assert_eq!(store.get_snapshot("src/other.ts").unwrap(), None);
    }

    #[test]
    fn a_hand_corrupted_category_column_surfaces_a_readable_error() {
        let store = SqliteStore::open_in_memory().unwrap();
        let c = candidate("AutoEntity");
        store.upsert_candidate(&c).unwrap();
        store
            .connection()
            .execute(
                "UPDATE candidate_rules SET category = 'nonsense' WHERE id = ?1",
                params![c.id],
            )
            .unwrap();

        let err = store.get_candidate(&c.id).unwrap_err();
        assert!(err.to_string().contains("nonsense"), "{err}");
    }
}
