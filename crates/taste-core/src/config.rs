//! Filesystem layout and the tunable constants of the scoring model.
//!
//! Nothing outside this module hardcodes a filename. [`Paths`] is the single
//! source of truth for where state lives and which editor files are synced.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Directory holding all project-local OpenTaste state.
pub const TASTE_DIR: &str = ".taste";
/// Git-tracked, human-reviewable confirmed rules.
pub const RULES_FILE: &str = "rules.json";
/// Local-only candidate cache and AST snapshots.
pub const CACHE_DB_FILE: &str = "cache.db";
/// Advisory lock guarding every read-modify-write of the rule set.
pub const LOCK_FILE: &str = ".lock";
/// Cross-project rules in the user's home directory.
pub const GLOBAL_DB_FILE: &str = "global.db";

/// Filename prefix of a generated Cursor rule file, `opentaste-<category>.mdc`.
pub const CURSOR_RULE_PREFIX: &str = "opentaste-";
/// Extension Cursor expects for rule files.
pub const CURSOR_RULE_EXTENSION: &str = "mdc";

/// Schema URL embedded in `.taste/rules.json`, served from `schema/v1.json`
/// in this repository.
pub const SCHEMA_URL: &str =
    "https://raw.githubusercontent.com/efadhemon/opentaste/main/schema/v1.json";
/// Schema version embedded in `.taste/rules.json`.
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Confidence a rule must reach before it is written to `.taste/rules.json`.
pub const PROMOTION_CONFIDENCE: f32 = 0.80;

/// Files that mark a directory as its own package inside a monorepo.
pub const PACKAGE_MANIFESTS: [&str; 4] = ["package.json", "Cargo.toml", "pyproject.toml", "go.mod"];
/// Independent observations a rule needs before promotion.
pub const PROMOTION_OCCURRENCES: u32 = 3;
/// Confidence below which a confirmed rule falls back to being a candidate.
pub const DEMOTION_CONFIDENCE: f32 = 0.50;

/// Resolved locations of every file OpenTaste reads or writes for one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub project_root: PathBuf,
    pub taste_dir: PathBuf,
    pub rules_json: PathBuf,
    pub cache_db: PathBuf,
    pub lock_file: PathBuf,
}

impl Paths {
    /// Derive all paths from a known project root without touching the disk.
    pub fn for_root(root: impl Into<PathBuf>) -> Self {
        let project_root = root.into();
        let taste_dir = project_root.join(TASTE_DIR);
        Paths {
            rules_json: taste_dir.join(RULES_FILE),
            cache_db: taste_dir.join(CACHE_DB_FILE),
            lock_file: taste_dir.join(LOCK_FILE),
            taste_dir,
            project_root,
        }
    }

    /// Walk up from `start` looking for a `.taste` directory, then a `.git` one.
    ///
    /// A `.taste` directory always wins, even if a `.git` directory is nearer,
    /// so nested checkouts keep using the project that was explicitly
    /// initialised.
    pub fn discover(start: impl AsRef<Path>) -> Result<Self> {
        let start = start.as_ref();
        let mut git_root: Option<PathBuf> = None;

        for dir in start.ancestors() {
            if dir.join(TASTE_DIR).is_dir() {
                return Ok(Paths::for_root(dir));
            }
            if git_root.is_none() && dir.join(".git").exists() {
                git_root = Some(dir.to_path_buf());
            }
        }

        git_root
            .map(Paths::for_root)
            .ok_or_else(|| Error::ProjectRootNotFound {
                start: start.to_path_buf(),
            })
    }

    /// Path to the cross-project database, `~/.taste/global.db`.
    pub fn global_db() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or(Error::HomeDirNotFound)?;
        Ok(home.join(TASTE_DIR).join(GLOBAL_DB_FILE))
    }

    /// Directory holding generated Cursor rule files.
    pub fn cursor_rules_dir(&self) -> PathBuf {
        self.project_root.join(".cursor").join("rules")
    }

    /// `CLAUDE.md` at the project root.
    pub fn claude_md(&self) -> PathBuf {
        self.project_root.join("CLAUDE.md")
    }

    /// `.windsurfrules` at the project root.
    pub fn windsurf_rules(&self) -> PathBuf {
        self.project_root.join(".windsurfrules")
    }

    /// `AGENTS.md` at the project root.
    pub fn agents_md(&self) -> PathBuf {
        self.project_root.join("AGENTS.md")
    }

    /// `.github/copilot-instructions.md`.
    pub fn copilot_instructions(&self) -> PathBuf {
        self.project_root
            .join(".github")
            .join("copilot-instructions.md")
    }

    /// Shared, developer-owned files the transpiler keeps one marker block in.
    ///
    /// Together with the generated files under [`Paths::cursor_rules_dir`],
    /// this is the complete set of transpiler targets. The file watcher and
    /// the editor extension exclude exactly these, so it lives in one place.
    pub fn marker_targets(&self) -> [PathBuf; 4] {
        [
            self.claude_md(),
            self.windsurf_rules(),
            self.agents_md(),
            self.copilot_instructions(),
        ]
    }

    /// Whether `path`, absolute or relative to the project root, is a file the
    /// transpiler writes. Reacting to these would make a watcher answer its
    /// own output.
    pub fn is_generated_target(&self, path: &Path) -> bool {
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        };
        if self.marker_targets().contains(&path) {
            return true;
        }
        let in_cursor_dir = path.parent() == Some(self.cursor_rules_dir().as_path());
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        in_cursor_dir
            && name.starts_with(CURSOR_RULE_PREFIX)
            && name.ends_with(&format!(".{CURSOR_RULE_EXTENSION}"))
    }

    /// The nearest directory strictly below the project root, walking up from
    /// the file at `relative`, that holds one of [`PACKAGE_MANIFESTS`]: a
    /// package of a monorepo. Returned relative to the root with `/`
    /// separators. `None` for a file that belongs to the root package.
    pub fn package_dir(&self, relative: &Path) -> Option<String> {
        let parent = relative.parent()?;
        parent
            .ancestors()
            .take_while(|dir| !dir.as_os_str().is_empty())
            .find(|dir| {
                PACKAGE_MANIFESTS
                    .iter()
                    .any(|manifest| self.project_root.join(dir).join(manifest).is_file())
            })
            .map(|dir| dir.to_string_lossy().replace('\\', "/"))
    }

    /// Render a path relative to the project root for display, falling back to
    /// the absolute path when it lies outside the project.
    pub fn relativize<'a>(&self, path: &'a Path) -> &'a Path {
        path.strip_prefix(&self.project_root).unwrap_or(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_transpiler_target_is_recognised_and_nothing_else_is() {
        let paths = Paths::for_root("/tmp/proj");
        for target in paths.marker_targets() {
            assert!(paths.is_generated_target(&target), "{target:?}");
        }
        assert!(paths.is_generated_target(Path::new("CLAUDE.md")));
        assert!(paths.is_generated_target(Path::new(".cursor/rules/opentaste-imports.mdc")));
        assert!(!paths.is_generated_target(Path::new(".cursor/rules/team.mdc")));
        assert!(!paths.is_generated_target(Path::new("src/opentaste-imports.mdc")));
        assert!(!paths.is_generated_target(Path::new("docs/CLAUDE.md")));
        assert!(!paths.is_generated_target(Path::new("src/app.ts")));
    }

    #[test]
    fn for_root_derives_every_path() {
        let paths = Paths::for_root("/tmp/proj");
        assert_eq!(paths.taste_dir, Path::new("/tmp/proj/.taste"));
        assert_eq!(paths.rules_json, Path::new("/tmp/proj/.taste/rules.json"));
        assert_eq!(paths.cache_db, Path::new("/tmp/proj/.taste/cache.db"));
        assert_eq!(paths.lock_file, Path::new("/tmp/proj/.taste/.lock"));
        assert_eq!(
            paths.copilot_instructions(),
            Path::new("/tmp/proj/.github/copilot-instructions.md")
        );
    }

    #[test]
    fn discover_prefers_taste_dir_over_nearer_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let outer = tmp.path();
        let inner = outer.join("packages").join("app");
        std::fs::create_dir_all(outer.join(TASTE_DIR)).unwrap();
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        std::fs::create_dir_all(inner.join("src")).unwrap();

        let found = Paths::discover(inner.join("src")).unwrap();
        assert_eq!(found.project_root, outer);
    }

    #[test]
    fn discover_falls_back_to_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let nested = root.join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();

        let found = Paths::discover(&nested).unwrap();
        assert_eq!(found.project_root, root);
    }

    #[test]
    fn discover_reports_missing_project() {
        let tmp = tempfile::tempdir().unwrap();
        let err = Paths::discover(tmp.path()).unwrap_err();
        assert!(matches!(err, Error::ProjectRootNotFound { .. }));
    }
}
