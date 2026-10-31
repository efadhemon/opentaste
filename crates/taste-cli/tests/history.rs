//! `taste scan --history` and `taste init` reading past commits.
//!
//! Every test builds its own repository, so each skip rule and the window are
//! exercised against real `git log` output rather than a mock.

use std::path::Path;
use std::process::{Command, Output};

/// A home directory for every `taste` this test binary runs, so personal
/// rules are written there and never to the real `~/.taste`.
fn test_home() -> &'static std::path::Path {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    HOME.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

fn taste(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taste"))
        .env("TASTE_HOME", test_home())
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run taste")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A repository where commits are made with full control of author, date and
/// message, and `taste init` has run with history turned off by the empty log.
struct Repo {
    dir: tempfile::TempDir,
}

/// How one commit is made.
struct By<'a> {
    author: &'a str,
    email: &'a str,
    days_ago: i64,
    message: &'a str,
}

const ADA: By<'static> = By {
    author: "Ada",
    email: "ada@example.com",
    days_ago: 1,
    message: "work",
};

impl Repo {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"], &[]);
        // Before any commit, so init's own history read finds nothing.
        assert!(taste(repo.root(), &["init", "--quiet"]).status.success());
        repo
    }

    /// A repository with commits but no OpenTaste yet, for `taste init`.
    fn uninitialised() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo { dir };
        repo.git(&["init", "-q", "-b", "main"], &[]);
        repo
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str], env: &[(&str, String)]) {
        let status = Command::new("git")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
            .current_dir(self.root())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git is installed");
        assert!(status.success(), "git {args:?}");
    }

    /// Write `files` and commit them as `by`.
    fn commit(&self, files: &[(&str, &str)], by: &By<'_>) {
        for (path, contents) in files {
            let path = self.root().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        self.git(&["add", "-A"], &[]);
        let when = (chrono::Utc::now() - chrono::Duration::days(by.days_ago)).to_rfc3339();
        let env = [
            ("GIT_AUTHOR_NAME", by.author.to_string()),
            ("GIT_AUTHOR_EMAIL", by.email.to_string()),
            ("GIT_COMMITTER_NAME", by.author.to_string()),
            ("GIT_COMMITTER_EMAIL", by.email.to_string()),
            ("GIT_AUTHOR_DATE", when.clone()),
            ("GIT_COMMITTER_DATE", when),
        ];
        self.git(&["commit", "-q", "--allow-empty", "-m", by.message], &env);
    }

    /// Introduce `print` in `name`, then correct it to a logger: one
    /// correction, two commits.
    fn correct(&self, name: &str, by: &By<'_>) {
        let before = "import logging\n\nlogger = logging.getLogger(__name__)\n\n\ndef sync(batch):\n    print(\"synced\", batch)\n";
        let after = before.replace("print(", "logger.info(");
        self.commit(
            &[(name, before)],
            &By {
                message: "add",
                ..*by
            },
        );
        self.commit(&[(name, &after)], by);
    }

    fn candidate_occurrences(&self) -> Vec<u32> {
        let output = taste(self.root(), &["status"]);
        stdout(&output)
            .lines()
            .filter(|line| line.contains("seen") && line.contains("time"))
            .filter_map(|line| {
                let seen = line.split("seen ").nth(1)?;
                seen.split_whitespace().next()?.parse().ok()
            })
            .collect()
    }
}

fn scan_history(repo: &Repo, commits: usize) -> String {
    let output = taste(repo.root(), &["scan", "--history", &commits.to_string()]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout(&output)
}

#[test]
fn a_correction_made_in_five_past_commits_counts_five_times() {
    let repo = Repo::new();
    for index in 0..5 {
        repo.correct(&format!("app/sync{index}.py"), &ADA);
    }
    let out = scan_history(&repo, 200);
    assert!(out.contains("Read 10 commits of history"), "{out}");
    assert!(out.contains("5 corrections came from history"), "{out}");
    assert!(out.contains("seen 5 times"), "{out}");
}

#[test]
fn mining_twice_or_mining_then_the_hook_counts_a_change_once() {
    let repo = Repo::new();
    repo.correct("app/sync.py", &ADA);
    scan_history(&repo, 200);
    let again = scan_history(&repo, 200);
    assert!(
        !again.contains("came from history"),
        "the cursor is at HEAD: {again}"
    );
    assert_eq!(repo.candidate_occurrences(), vec![1]);

    let hook = stdout(&taste(repo.root(), &["learn", "--from-git-head"]));
    assert!(hook.contains("Already learned"), "{hook}");
    assert_eq!(repo.candidate_occurrences(), vec![1]);
}

#[test]
fn the_cursor_makes_the_next_run_read_only_newer_commits() {
    let repo = Repo::new();
    repo.correct("app/a.py", &ADA);
    scan_history(&repo, 200);
    repo.correct("app/b.py", &ADA);
    let out = scan_history(&repo, 200);
    assert!(out.contains("Read 2 commits of history"), "{out}");
    assert!(out.contains("seen 2 times"), "{out}");
}

#[test]
fn the_window_stops_at_whichever_limit_comes_first() {
    let repo = Repo::new();
    for index in 0..4 {
        repo.correct(&format!("app/s{index}.py"), &ADA);
    }
    // `--history 2`: the two newest commits, which are one correction.
    let out = scan_history(&repo, 2);
    assert!(out.contains("Read 2 commits of history"), "{out}");
    assert!(out.contains("1 correction came from history"), "{out}");

    // `taste init`'s window also stops at 180 days.
    let old = Repo::uninitialised();
    old.correct(
        "app/old.py",
        &By {
            days_ago: 400,
            ..ADA
        },
    );
    old.correct("app/new.py", &ADA);
    let init = stdout(&taste(old.root(), &["init"]));
    assert!(init.contains("Read 2 commits of history"), "{init}");
    assert!(init.contains("1 correction came from history"), "{init}");
}

#[test]
fn bots_reverts_and_sweeps_are_not_learned_from() {
    let repo = Repo::new();
    repo.correct(
        "app/bot.py",
        &By {
            author: "dependabot[bot]",
            email: "49699333+dependabot[bot]@users.noreply.github.com",
            ..ADA
        },
    );
    repo.correct(
        "app/revert.py",
        &By {
            message: "Revert \"Use print\"\n\nThis reverts commit 1234567.",
            ..ADA
        },
    );
    let sweep: Vec<(String, String)> = (0..51)
        .map(|i| (format!("fmt/f{i}.txt"), format!("{i}\n")))
        .collect();
    let mut files: Vec<(&str, &str)> = sweep
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    let before = "import logging\n\nlogger = logging.getLogger(__name__)\n\n\ndef sync(batch):\n    print(\"synced\", batch)\n";
    let after = before.replace("print(", "logger.info(");
    repo.commit(&[("app/sweep.py", before)], &ADA);
    files.push(("app/sweep.py", &after));
    repo.commit(&files, &ADA);

    let out = scan_history(&repo, 200);
    // Both of the bot's commits, the revert, and the sweep. What is left is
    // the two commits that added a file, which correct nothing.
    assert!(
        out.contains("Read 2 commits of history (4 skipped: 2 by bots, 1 reverts, 1 touching over 50 files); 0 corrections"),
        "{out}"
    );
}

#[test]
fn history_never_decays_a_rule() {
    let repo = Repo::new();
    // A rule confirmed by the developer, then a past commit full of its
    // anti-pattern: written before the rule existed, so it is not a contradiction.
    let rules = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/efadhemon/opentaste/main/schema/v1.json",
        "version": "1.0.0",
        "project_context": { "detected_stack": [], "total_files_scanned": 0, "last_updated": "2026-01-01T00:00:00Z" },
        "rules": [{
            "id": "style-python-library-code-log-through-logging-000001",
            "category": "code-style",
            "scope": "**/*.py",
            "confidence": 0.95,
            "occurrences": 3,
            "enabled": true,
            "source": "mcp-feedback",
            "instruction": "In Python library code, log through the `logging` module.",
            "anti_pattern": "print(\"synced\", batch)",
            "preferred_pattern": "logger.info(\"synced\", batch)",
            "updated_at": "2026-01-01T00:00:00Z"
        }]
    });
    repo.commit(&[("app/a.py", "def a():\n    pass\n")], &ADA);
    repo.commit(
        &[("app/a.py", "def a(batch):\n    print(\"synced\", batch)\n")],
        &ADA,
    );
    std::fs::create_dir_all(repo.root().join(".taste")).unwrap();
    std::fs::write(repo.root().join(".taste/rules.json"), rules.to_string()).unwrap();
    scan_history(&repo, 200);
    let after: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo.root().join(".taste/rules.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(after["rules"][0]["confidence"], 0.95, "{after}");
}

#[test]
fn status_reports_how_much_history_was_read() {
    let repo = Repo::new();
    repo.correct("app/a.py", &ADA);
    let before = stdout(&taste(repo.root(), &["status"]));
    assert!(before.contains("History: not read yet"), "{before}");
    scan_history(&repo, 200);
    let after = stdout(&taste(repo.root(), &["status"]));
    assert!(after.contains("History: 2 commits read, up to "), "{after}");
}

#[test]
fn a_standalone_scan_that_confirms_a_rule_syncs_the_editor_files() {
    let repo = Repo::new();
    for index in 0..10 {
        repo.commit(
            &[(
                &format!("src/a{index}.ts") as &str,
                "import { a } from \"@/lib/a\";\nexport const x = a;\n",
            )],
            &ADA,
        );
    }
    let out = stdout(&taste(repo.root(), &["scan"]));
    assert!(out.contains("confirmed"), "{out}");
    let claude = std::fs::read_to_string(repo.root().join("CLAUDE.md")).unwrap_or_default();
    assert!(claude.contains("path aliases"), "{out}\n---\n{claude}");
}

#[test]
fn a_run_interrupted_before_its_cursor_moved_resumes_without_counting_twice() {
    let repo = Repo::new();
    repo.correct("app/a.py", &ADA);
    repo.correct("app/b.py", &ADA);
    scan_history(&repo, 200);
    assert_eq!(repo.candidate_occurrences(), vec![2]);

    // As if the run stopped after recording everything but before the cursor
    // reached HEAD: the cursor is back at the first commit.
    let first = String::from_utf8(
        Command::new("git")
            .args(["rev-list", "--max-parents=0", "HEAD"])
            .current_dir(repo.root())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let store =
        taste_core::RuleStore::open(taste_core::config::Paths::for_root(repo.root())).unwrap();
    store
        .candidates()
        .set_meta("history.cursor", first.trim())
        .unwrap();
    drop(store);

    let resumed = scan_history(&repo, 200);
    assert!(resumed.contains("Read 3 commits of history"), "{resumed}");
    assert!(
        resumed.contains("0 corrections came from history"),
        "{resumed}"
    );
    assert_eq!(
        repo.candidate_occurrences(),
        vec![2],
        "nothing counted twice"
    );
}

#[test]
fn a_past_correction_the_code_has_since_reversed_is_left_out() {
    let repo = Repo::new();
    // Once, a declaration was turned into an arrow...
    repo.commit(
        &[("src/a.ts", "export function load() {\n  return 1;\n}\n")],
        &ADA,
    );
    repo.commit(
        &[("src/a.ts", "export const load = () => {\n  return 1;\n};\n")],
        &ADA,
    );
    // ...but today the code is written with declarations.
    let today: Vec<(String, String)> = (0..6)
        .map(|i| {
            (
                format!("src/m{i}.ts"),
                format!("export function m{i}() {{\n  return {i};\n}}\n"),
            )
        })
        .collect();
    let files: Vec<(&str, &str)> = today
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();
    repo.commit(&files, &ADA);

    let out = scan_history(&repo, 200);
    assert!(out.contains("0 corrections came from history"), "{out}");
    assert!(out.contains("left out 1 past correction"), "{out}");
}
