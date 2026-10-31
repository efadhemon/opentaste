//! `~/.taste/global.db`: the developer's personal rules, across projects.
//!
//! Two things live here, both on the developer's own machine and nowhere
//! else:
//!
//! - **Personal rules**, which the developer marks as theirs explicitly
//!   (`taste global add`, or `record_coding_taste` with `"global": true`).
//!   Nothing becomes personal on its own.
//! - **Sightings**: which projects confirmed which rule, so a rule confirmed
//!   in two or more projects can be *suggested* as personal.
//!
//! The project cache's tables (candidates, snapshots, learned changes) are
//! always about one project, so they are not repeated here. Writes take a
//! lock in `~/.taste/`, separate from any project's, so two projects syncing
//! at once serialise rather than interleave.

use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};

use super::lock::TasteLock;
use crate::config::{GLOBAL_DB_FILE, LOCK_FILE, TASTE_DIR};
use crate::error::Result;
use crate::models::TasteRule;

const SCHEMA_VERSION: i32 = 1;

/// The personal-rule database in a home directory.
#[derive(Debug)]
pub struct GlobalStore {
    conn: Connection,
    lock_file: PathBuf,
}

/// A rule confirmed in more than one project that is not personal yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub rule: TasteRule,
    /// Project roots it was confirmed in, sorted.
    pub projects: Vec<String>,
}

impl GlobalStore {
    /// Open, creating as needed, `<home>/.taste/global.db`.
    pub fn open(home: &Path) -> Result<Self> {
        let dir = home.join(TASTE_DIR);
        std::fs::create_dir_all(&dir).map_err(|source| crate::error::Error::Io {
            path: dir.clone(),
            source,
        })?;
        let conn = Connection::open(dir.join(GLOBAL_DB_FILE))?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let store = GlobalStore {
            conn,
            lock_file: dir.join(LOCK_FILE),
        };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let current: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if current < 1 {
            self.conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS personal_rules (
                    id TEXT PRIMARY KEY,
                    rule TEXT NOT NULL,
                    added_from TEXT NOT NULL,
                    added_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS seen_in (
                    rule_id TEXT NOT NULL,
                    project TEXT NOT NULL,
                    rule TEXT NOT NULL,
                    seen_at TEXT NOT NULL,
                    PRIMARY KEY (rule_id, project)
                );
                "#,
            )?;
        }
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    }

    fn lock(&self) -> Result<TasteLock> {
        TasteLock::acquire(&self.lock_file)
    }

    /// Make `rule` personal. Its snippets are code from one project, so they
    /// are kept only when `keep_snippets` says so. Returns `false` when the
    /// rule was already personal (it is updated either way).
    pub fn add(&self, rule: &TasteRule, keep_snippets: bool, from: &Path) -> Result<bool> {
        let _guard = self.lock()?;
        let mut rule = rule.clone();
        if !keep_snippets {
            rule.anti_pattern = None;
            rule.preferred_pattern = None;
        }
        let existed = self.get(&rule.id)?.is_some();
        self.conn.execute(
            r#"
            INSERT INTO personal_rules (id, rule, added_from, added_at) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET rule = excluded.rule
            "#,
            params![
                rule.id,
                serde_json::to_string(&rule)?,
                path_key(from),
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(!existed)
    }

    /// Stop treating a rule as personal. Returns `false` when it was not.
    pub fn remove(&self, id: &str) -> Result<bool> {
        let _guard = self.lock()?;
        let removed = self
            .conn
            .execute("DELETE FROM personal_rules WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    pub fn get(&self, id: &str) -> Result<Option<TasteRule>> {
        let rule: Option<String> = self
            .conn
            .query_row(
                "SELECT rule FROM personal_rules WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match rule {
            Some(json) => Some(serde_json::from_str(&json)?),
            None => None,
        })
    }

    /// Every personal rule, ordered by identifier.
    pub fn list(&self) -> Result<Vec<TasteRule>> {
        let mut statement = self
            .conn
            .prepare("SELECT rule FROM personal_rules ORDER BY id")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut rules = Vec::new();
        for row in rows {
            rules.push(serde_json::from_str(&row?)?);
        }
        Ok(rules)
    }

    /// Remember that `project` confirmed `rule`.
    pub fn record_seen(&self, rule: &TasteRule, project: &Path) -> Result<()> {
        let _guard = self.lock()?;
        let mut rule = rule.clone();
        rule.anti_pattern = None;
        rule.preferred_pattern = None;
        self.conn.execute(
            r#"
            INSERT INTO seen_in (rule_id, project, rule, seen_at) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(rule_id, project) DO UPDATE SET rule = excluded.rule, seen_at = excluded.seen_at
            "#,
            params![
                rule.id,
                path_key(project),
                serde_json::to_string(&rule)?,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(())
    }

    /// Rules confirmed in at least two projects that are not personal yet,
    /// most widely confirmed first.
    pub fn suggestions(&self) -> Result<Vec<Suggestion>> {
        let mut statement = self.conn.prepare(
            r#"
            SELECT rule_id, project, rule FROM seen_in
            WHERE rule_id NOT IN (SELECT id FROM personal_rules)
            ORDER BY rule_id, project
            "#,
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut found: Vec<Suggestion> = Vec::new();
        for row in rows {
            let (id, project, json) = row?;
            match found.last_mut() {
                Some(last) if last.rule.id == id => last.projects.push(project),
                _ => found.push(Suggestion {
                    rule: serde_json::from_str(&json)?,
                    projects: vec![project],
                }),
            }
        }
        found.retain(|suggestion| suggestion.projects.len() >= 2);
        found.sort_by(|a, b| {
            b.projects
                .len()
                .cmp(&a.projects.len())
                .then_with(|| a.rule.id.cmp(&b.rule.id))
        });
        Ok(found)
    }
}

/// A project root as stored: absolute, with forward slashes.
fn path_key(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    absolute.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RuleCategory, RuleSource};

    fn rule(instruction: &str) -> TasteRule {
        let mut candidate = crate::models::CandidateRule::new(
            RuleCategory::CodeStyle,
            "**/*.ts",
            RuleSource::Manual,
            instruction,
        );
        candidate.anti_pattern = Some("console.log(x)".into());
        candidate.preferred_pattern = Some("logger.info(x)".into());
        candidate.into_rule()
    }

    #[test]
    fn the_database_is_created_in_the_home_directory_and_reopens() {
        let home = tempfile::tempdir().unwrap();
        let store = GlobalStore::open(home.path()).unwrap();
        store
            .add(&rule("Log, do not print."), false, home.path())
            .unwrap();
        drop(store);
        assert!(home.path().join(".taste/global.db").is_file());
        let again = GlobalStore::open(home.path()).unwrap();
        assert_eq!(again.list().unwrap().len(), 1);
        let version: i32 = again
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn snippets_are_kept_only_when_asked_and_a_rule_is_removed() {
        let home = tempfile::tempdir().unwrap();
        let store = GlobalStore::open(home.path()).unwrap();
        let plain = rule("Log, do not print.");
        assert!(store.add(&plain, false, home.path()).unwrap());
        assert!(
            !store.add(&plain, false, home.path()).unwrap(),
            "already personal"
        );
        assert_eq!(store.get(&plain.id).unwrap().unwrap().anti_pattern, None);
        let kept = rule("Prefer early returns.");
        store.add(&kept, true, home.path()).unwrap();
        assert!(store.get(&kept.id).unwrap().unwrap().anti_pattern.is_some());

        assert!(store.remove(&plain.id).unwrap());
        assert!(!store.remove(&plain.id).unwrap());
        assert_eq!(
            store.list().unwrap(),
            vec![store.get(&kept.id).unwrap().unwrap()]
        );
    }

    #[test]
    fn a_rule_seen_in_two_projects_is_suggested_until_it_is_personal() {
        let home = tempfile::tempdir().unwrap();
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let store = GlobalStore::open(home.path()).unwrap();
        let shared = rule("Prefer named exports.");
        let once = rule("Prefer interfaces.");
        store.record_seen(&shared, a.path()).unwrap();
        store.record_seen(&shared, a.path()).unwrap();
        store.record_seen(&once, a.path()).unwrap();
        assert!(
            store.suggestions().unwrap().is_empty(),
            "one project is not a pattern"
        );

        store.record_seen(&shared, b.path()).unwrap();
        let suggestions = store.suggestions().unwrap();
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].rule.id, shared.id);
        assert_eq!(suggestions[0].projects.len(), 2);
        assert_eq!(
            suggestions[0].rule.anti_pattern, None,
            "sightings keep no code"
        );

        store.add(&shared, false, a.path()).unwrap();
        assert!(store.suggestions().unwrap().is_empty());
    }

    #[test]
    fn writes_from_two_projects_at_once_all_land() {
        let home = tempfile::tempdir().unwrap();
        GlobalStore::open(home.path()).unwrap();
        let threads: Vec<_> = (0..2)
            .map(|worker| {
                let home = home.path().to_path_buf();
                std::thread::spawn(move || {
                    let store = GlobalStore::open(&home).unwrap();
                    for index in 0..25 {
                        store
                            .add(&rule(&format!("Rule {worker} {index}.")), false, &home)
                            .unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(
            GlobalStore::open(home.path())
                .unwrap()
                .list()
                .unwrap()
                .len(),
            50
        );
    }
}
