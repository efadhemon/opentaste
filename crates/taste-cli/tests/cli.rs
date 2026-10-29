//! Drive the real `taste` binary. Cargo guarantees `CARGO_BIN_EXE_taste`.

use std::path::Path;
use std::process::{Command, Output};

fn taste(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taste"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run taste")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn outside_any_project_taste_fails_and_names_taste_init() {
    let dir = tempfile::tempdir().unwrap();
    for command in ["status", "scan", "sync", "mcp"] {
        let output = taste(dir.path(), &[command]);
        assert!(!output.status.success(), "{command}");
        assert!(
            stderr(&output).contains("taste init"),
            "{command}: {}",
            stderr(&output)
        );
        assert!(output.stdout.is_empty(), "{command}");
    }
}

#[test]
fn a_bare_git_repository_is_not_set_up_until_init_runs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let output = taste(dir.path(), &["status"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("taste init"),
        "{}",
        stderr(&output)
    );
    assert!(
        !dir.path().join(".taste").exists(),
        "status must not create .taste/"
    );
}

#[test]
fn version_prints_the_crate_version() {
    let dir = tempfile::tempdir().unwrap();
    let output = taste(dir.path(), &["--version"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")));
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git is installed");
    assert!(status.success(), "git {args:?}");
}

fn git_repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

#[test]
fn init_keeps_other_mcp_servers_and_an_existing_hook() {
    let dir = git_repo();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".cursor")).unwrap();
    std::fs::write(
        root.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"github":{"command":"gh-mcp","args":["serve"]}}}"#,
    )
    .unwrap();
    let hook = root.join(".git/hooks/post-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'team hook'\n").unwrap();

    let output = taste(root, &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));

    let mcp: serde_json::Value = serde_json::from_str(&read(root, ".cursor/mcp.json")).unwrap();
    assert_eq!(mcp["mcpServers"]["github"]["command"], "gh-mcp");
    assert_eq!(mcp["mcpServers"]["opentaste"]["args"][0], "mcp");

    let hook_text = std::fs::read_to_string(&hook).unwrap();
    assert!(
        hook_text.starts_with("#!/bin/sh\necho 'team hook'\n"),
        "{hook_text}"
    );
    assert!(
        hook_text.contains("taste learn --from-git-head"),
        "{hook_text}"
    );
    assert!(root.join(".taste/rules.json").is_file());
    assert!(root.join(".taste/cache.db").is_file());
}

#[cfg(unix)]
#[test]
fn a_new_hook_is_executable() {
    use std::os::unix::fs::PermissionsExt;
    let dir = git_repo();
    assert!(taste(dir.path(), &["init"]).status.success());
    let mode = std::fs::metadata(dir.path().join(".git/hooks/post-commit"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "{mode:o}");
}

#[test]
fn init_twice_is_a_no_op_that_says_so() {
    let dir = git_repo();
    let root = dir.path();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    assert!(taste(root, &["init"]).status.success());
    let files = [
        ".gitignore",
        ".cursor/mcp.json",
        ".git/hooks/post-commit",
        ".taste/rules.json",
    ];
    let first: Vec<String> = files.iter().map(|path| read(root, path)).collect();

    let second = taste(root, &["init"]);
    assert!(second.status.success(), "{}", stderr(&second));
    let again: Vec<String> = files.iter().map(|path| read(root, path)).collect();
    assert_eq!(first, again, "a second init must not change any file");
    assert!(
        stdout(&second).contains("Nothing to do"),
        "{}",
        stdout(&second)
    );

    let gitignore = read(root, ".gitignore");
    for entry in [
        ".taste/cache.db",
        ".taste/cache.db-wal",
        ".taste/cache.db-shm",
        ".taste/.lock",
    ] {
        assert_eq!(
            gitignore.lines().filter(|line| *line == entry).count(),
            1,
            "{entry}"
        );
    }
    assert_eq!(
        read(root, ".git/hooks/post-commit")
            .matches(">>> opentaste >>>")
            .count(),
        1
    );
}

#[test]
fn init_leaves_invalid_mcp_json_untouched_and_explains() {
    let dir = git_repo();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".cursor")).unwrap();
    let broken = "{ \"mcpServers\": { \"github\": ";
    std::fs::write(root.join(".cursor/mcp.json"), broken).unwrap();

    let output = taste(root, &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(read(root, ".cursor/mcp.json"), broken);
    let text = stdout(&output);
    assert!(text.contains("not valid JSON"), "{text}");
    assert!(text.contains("\"opentaste\""), "{text}");
    assert!(
        root.join(".taste/rules.json").is_file(),
        "the other steps still ran"
    );
}

#[test]
fn init_outside_git_skips_the_hook_and_still_sets_up() {
    let dir = tempfile::tempdir().unwrap();
    let output = taste(dir.path(), &["init"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("not a git repository"),
        "{}",
        stdout(&output)
    );
    assert!(dir.path().join(".taste/rules.json").is_file());
    // And afterwards the project is found through `.taste/`.
    assert!(taste(dir.path(), &["sync"]).status.success());
}

#[test]
fn a_hook_manager_owning_the_hooks_is_not_overridden() {
    let dir = git_repo();
    git(dir.path(), &["config", "core.hooksPath", ".husky/_"]);
    let output = taste(dir.path(), &["init"]);
    assert!(output.status.success());
    assert!(
        stdout(&output).contains("core.hooksPath"),
        "{}",
        stdout(&output)
    );
    assert!(!dir.path().join(".husky").exists());
    assert!(!dir.path().join(".git/hooks/post-commit").exists());
}

#[test]
fn quiet_init_prints_nothing() {
    let dir = git_repo();
    let output = taste(dir.path(), &["init", "--quiet"]);
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
}

fn taste_with_stdin(dir: &Path, args: &[&str], stdin: &str) -> Output {
    use std::io::Write;
    let mut child = Command::new(env!("CARGO_BIN_EXE_taste"))
        .args(args)
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn initialised() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(taste(dir.path(), &["init", "--quiet"]).status.success());
    dir
}

const ENTITY_BEFORE: &str = "@Entity(\"users\")\nexport class User {}\n";
const ENTITY_AFTER: &str = "@AutoEntity()\nexport class User {}\n";

fn delta(before: &str, after: &str) -> String {
    serde_json::json!({ "before": before, "after": after }).to_string()
}

fn learn_delta(dir: &Path, file: &str, before: &str, after: &str) -> serde_json::Value {
    let output = taste_with_stdin(dir, &["learn-delta", "--file", file], &delta(before, after));
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1, "exactly one line: {text:?}");
    assert!(text.ends_with('\n'));
    serde_json::from_str(&text).unwrap()
}

#[test]
fn learn_delta_reports_nothing_learned_as_two_empty_arrays() {
    let dir = initialised();
    let value = learn_delta(dir.path(), "src/app.ts", "const a = 1;\n", "const a = 2;\n");
    assert_eq!(
        value,
        serde_json::json!({ "promoted": [], "candidates": [] })
    );
}

#[test]
fn learn_delta_follows_the_contract_from_candidate_to_promotion() {
    let dir = initialised();
    let file = "src/users/user.entity.ts";

    let first = learn_delta(dir.path(), file, ENTITY_BEFORE, ENTITY_AFTER);
    assert_eq!(first["promoted"], serde_json::json!([]));
    let candidates = first["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 1, "{first}");
    let keys: Vec<&str> = candidates[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, vec!["id", "confidence", "instruction"]);
    assert!(candidates[0]["instruction"]
        .as_str()
        .unwrap()
        .contains("@AutoEntity"));

    let mut last = first;
    for _ in 0..5 {
        last = learn_delta(dir.path(), file, ENTITY_BEFORE, ENTITY_AFTER);
        if !last["promoted"].as_array().unwrap().is_empty() {
            break;
        }
    }
    let promoted = last["promoted"].as_array().unwrap();
    assert_eq!(promoted.len(), 1, "{last}");
    let rule = promoted[0].as_object().unwrap();
    let mut keys: Vec<&str> = rule.keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "anti_pattern",
            "category",
            "confidence",
            "enabled",
            "id",
            "instruction",
            "occurrences",
            "preferred_pattern",
            "scope",
            "source",
            "updated_at"
        ]
    );
    assert_eq!(rule["source"], "ast-diff");
    assert_eq!(rule["scope"], "src/**/*.entity.ts");
    assert_eq!(rule["category"], "architecture");

    // It is the same object `.taste/rules.json` now holds, and it reached the editors.
    let file_rules: serde_json::Value =
        serde_json::from_str(&read(dir.path(), ".taste/rules.json")).unwrap();
    assert_eq!(file_rules["rules"][0], promoted[0]);
    assert!(read(dir.path(), "CLAUDE.md").contains("@AutoEntity"));
}

#[test]
fn learn_delta_errors_go_to_stderr_with_an_empty_stdout() {
    let dir = initialised();
    for bad in [
        "not json",
        "{\"before\": \"a\"}",
        "{\"before\":\"a\",\"after\":\"b\",\"x\":1}",
    ] {
        let output = taste_with_stdin(dir.path(), &["learn-delta", "--file", "src/a.ts"], bad);
        assert!(!output.status.success(), "{bad}");
        assert!(output.stdout.is_empty(), "{bad}: {}", stdout(&output));
        assert!(!stderr(&output).is_empty());
    }
}

#[test]
fn learn_delta_ignores_quiet_because_its_stdout_is_the_contract() {
    let dir = initialised();
    let output = taste_with_stdin(
        dir.path(),
        &["learn-delta", "--quiet", "--file", "src/a.ts"],
        &delta("a", "b"),
    );
    assert!(output.status.success());
    assert_eq!(stdout(&output).trim(), r#"{"promoted":[],"candidates":[]}"#);
}

fn commit(dir: &Path, path: &str, contents: &str) {
    let file = dir.join(path);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, contents).unwrap();
    git(dir, &["add", "-A"]);
    // The installed hook would run whatever `taste` is on the PATH; keep it out.
    git(
        dir,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            path,
        ],
    );
}

#[test]
fn learn_from_git_head_records_what_the_last_commit_changed() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());
    commit(root, "src/users/user.entity.ts", ENTITY_BEFORE);
    // The first commit has no parent; it must not fail.
    assert!(taste(root, &["learn", "--from-git-head"]).status.success());

    commit(root, "src/users/user.entity.ts", ENTITY_AFTER);
    let output = taste(root, &["learn", "--from-git-head"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("candidate"), "{}", stdout(&output));
}

#[test]
fn learn_from_git_head_decays_a_rule_whose_anti_pattern_the_commit_added() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());
    commit(
        root,
        "src/app.ts",
        "export function a() {\n  return 1;\n}\n",
    );
    let rules = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/efadhemon/opentaste/main/schema/v1.json",
        "version": "1.0.0",
        "project_context": { "detected_stack": [], "total_files_scanned": 0, "last_updated": "2026-10-29T00:00:00Z" },
        "rules": [{
            "id": "errors-domainerror-000001",
            "category": "error-handling",
            "scope": "src/**/*.ts",
            "confidence": 0.95,
            "occurrences": 3,
            "enabled": true,
            "source": "mcp-feedback",
            "instruction": "Throw DomainError instead of Error.",
            "anti_pattern": "throw new Error(\"missing\")",
            "updated_at": "2026-10-29T00:00:00Z"
        }]
    });
    std::fs::write(root.join(".taste/rules.json"), rules.to_string()).unwrap();

    commit(
        root,
        "src/app.ts",
        "export function a() {\n  return 1;\n}\nexport function b() {\n  throw new Error(\"missing\");\n}\n",
    );
    let output = taste(root, &["learn", "--from-git-head"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("decayed"), "{}", stdout(&output));
    let after: serde_json::Value = serde_json::from_str(&read(root, ".taste/rules.json")).unwrap();
    let confidence = after["rules"][0]["confidence"].as_f64().unwrap();
    assert!(confidence < 0.95, "{after}");
}

fn amend(dir: &Path) {
    git(
        dir,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "--amend",
            "-m",
            "amended",
        ],
    );
}

#[test]
fn learning_the_same_commit_again_does_not_count_its_correction_twice() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());
    commit(root, "src/users/user.entity.ts", ENTITY_BEFORE);
    commit(root, "src/users/user.entity.ts", ENTITY_AFTER);

    let first = taste(root, &["learn", "--from-git-head"]);
    assert!(stdout(&first).contains("candidate"), "{}", stdout(&first));
    let status_after_first = stdout(&taste(root, &["status"]));

    let again = taste(root, &["learn", "--from-git-head"]);
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(
        stdout(&again).contains("Already learned"),
        "{}",
        stdout(&again)
    );
    assert_eq!(stdout(&taste(root, &["status"])), status_after_first);
}

#[test]
fn an_amended_commit_with_the_same_change_is_not_learned_from_again() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());
    commit(root, "src/users/user.entity.ts", ENTITY_BEFORE);
    commit(root, "src/users/user.entity.ts", ENTITY_AFTER);
    assert!(taste(root, &["learn", "--from-git-head"]).status.success());
    let status_before_amend = stdout(&taste(root, &["status"]));

    // `--amend` makes a new commit and fires post-commit again.
    amend(root);
    let output = taste(root, &["learn", "--from-git-head"]);
    assert!(
        stdout(&output).contains("Already learned"),
        "{}",
        stdout(&output)
    );
    assert_eq!(stdout(&taste(root, &["status"])), status_before_amend);
}

const SERVICE: &str = "src/users/user.service.ts";
const BASE: &str = "export const ping = () => 1;\n";

fn find_user(error_class: &str, message: &str) -> String {
    format!(
        "export function findUser(id: string) {{\n  if (!id) throw new {error_class}(\"{message}\");\n  return db.get(id);\n}}\n"
    )
}

/// The candidate's occurrence count, as `taste status` prints it.
fn seen(root: &Path) -> String {
    let status = stdout(&taste(root, &["status"]));
    let line = status
        .lines()
        .find(|line| line.contains("· seen"))
        .unwrap_or_else(|| panic!("no candidate in:\n{status}"));
    line.split("· seen ").nth(1).unwrap().to_string()
}

#[test]
fn a_correction_the_extension_learned_is_not_counted_again_when_committed() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());
    let ai = find_user("Error", "missing id");
    let fixed = find_user("NotFoundError", "missing id");

    // The AI's block is committed, then corrected in the editor and saved.
    commit(root, SERVICE, BASE);
    commit(root, SERVICE, &format!("{BASE}{ai}"));
    learn_delta(root, SERVICE, &ai, &fixed);
    assert!(seen(root).starts_with("1 time"), "{}", seen(root));

    // The correction is committed, and the hook sees the same edit.
    commit(root, SERVICE, &format!("{BASE}{fixed}"));
    let output = taste(root, &["learn", "--from-git-head"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("already    learned from the editor"),
        "{}",
        stdout(&output)
    );
    assert!(seen(root).starts_with("1 time"), "{}", seen(root));
}

#[test]
fn two_different_corrections_of_the_same_kind_still_count_twice() {
    let dir = git_repo();
    let root = dir.path();
    assert!(taste(root, &["init", "--quiet"]).status.success());

    // The extension learns one correction...
    commit(root, SERVICE, BASE);
    commit(
        root,
        SERVICE,
        &format!("{BASE}{}", find_user("Error", "missing id")),
    );
    learn_delta(
        root,
        SERVICE,
        &find_user("Error", "missing id"),
        &find_user("NotFoundError", "missing id"),
    );

    // ...and a commit carries a different one, which the editor never saw.
    commit(
        root,
        SERVICE,
        &format!("{BASE}{}", find_user("Error", "no user")),
    );
    commit(
        root,
        SERVICE,
        &format!("{BASE}{}", find_user("NotFoundError", "no user")),
    );
    assert!(taste(root, &["learn", "--from-git-head"]).status.success());
    assert!(seen(root).starts_with("2 times"), "{}", seen(root));
}

#[test]
fn learn_without_a_commit_fails_cleanly() {
    let dir = git_repo();
    assert!(taste(dir.path(), &["init", "--quiet"]).status.success());
    let output = taste(dir.path(), &["learn", "--from-git-head"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("no commit"), "{}", stderr(&output));
}

#[test]
fn status_shows_rules_candidates_scan_counts_and_target_freshness() {
    let dir = initialised();
    let root = dir.path();
    for _ in 0..6 {
        learn_delta(
            root,
            "src/users/user.entity.ts",
            ENTITY_BEFORE,
            ENTITY_AFTER,
        );
    }
    learn_delta(
        root,
        "src/app.ts",
        "interface User {\n  id: string;\n}\n",
        "type User = {\n  id: string;\n};\n",
    );
    for index in 0..3 {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join(format!("src/f{index}.ts")),
            "export const x = 1;\n",
        )
        .unwrap();
    }
    assert!(taste(root, &["scan", "--quiet"]).status.success());

    let output = taste_env(root, &["status"], &[("TASTE_ASCII", "1")]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("Confirmed rules (1)"), "{text}");
    assert!(
        text.contains("[x]") && text.contains("@AutoEntity"),
        "{text}"
    );
    assert!(
        text.contains("Candidates (1)") && text.contains("[?]"),
        "{text}"
    );
    assert!(text.contains("exports"), "scan counts: {text}");
    assert!(text.contains("up to date CLAUDE.md"), "{text}");

    std::fs::write(root.join("CLAUDE.md"), "edited by hand\n").unwrap();
    let stale = stdout(&taste(root, &["status"]));
    assert!(stale.contains("stale      CLAUDE.md"), "{stale}");
    assert!(stale.contains("taste sync"), "{stale}");
}

fn taste_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taste"))
        .args(args)
        .current_dir(dir)
        .envs(env.iter().copied())
        .output()
        .expect("run taste")
}

#[test]
fn sync_names_the_targets_it_changed() {
    let dir = initialised();
    for _ in 0..6 {
        learn_delta(
            dir.path(),
            "src/users/user.entity.ts",
            ENTITY_BEFORE,
            ENTITY_AFTER,
        );
    }
    std::fs::remove_file(dir.path().join("AGENTS.md")).unwrap();
    let text = stdout(&taste(dir.path(), &["sync"]));
    assert!(text.contains("updated    AGENTS.md"), "{text}");
    assert!(text.contains("up to date CLAUDE.md"), "{text}");
    let again = stdout(&taste(dir.path(), &["sync"]));
    assert!(again.contains("already up to date"), "{again}");
}

#[test]
fn mcp_answers_a_ping_on_stdout_and_nothing_else() {
    let dir = initialised();
    let output = taste_with_stdin(
        dir.path(),
        &["mcp"],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n",
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n"
    );
}

/// The wiring end to end: a saved source file triggers a rescan. inotify
/// delivers promptly, so this runs on Linux; FSEvents on macOS and
/// ReadDirectoryChangesW on Windows are covered by the unit tests and by hand.
#[cfg(target_os = "linux")]
#[test]
fn watch_rescans_after_a_source_file_is_saved() {
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    let dir = initialised();
    let mut child = Command::new(env!("CARGO_BIN_EXE_taste"))
        .arg("watch")
        .current_dir(dir.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = sender.send(line);
        }
    });
    let wait_for = |needle: &str| -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(line) = receiver.recv_timeout(Duration::from_millis(100)) {
                if line.contains(needle) {
                    return true;
                }
            }
        }
        false
    };
    assert!(wait_for("Watching"), "the watcher started");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/new.ts"), "export const a = 1;\n").unwrap();
    let rescanned = wait_for("rescanning");
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(rescanned, "saving a .ts file triggers a rescan");
}

#[test]
fn a_reader_that_closes_early_does_not_make_taste_panic() {
    let dir = initialised();
    let mut child = Command::new(env!("CARGO_BIN_EXE_taste"))
        .arg("status")
        .current_dir(dir.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Close the only read end before taste has opened its store, so every
    // write it makes hits a broken pipe, as `taste status | head -1` does.
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    let stderr = stderr(&output);
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(output.status.success(), "{stderr}");
}

#[test]
fn targets_json_lists_every_file_sync_writes() {
    let dir = initialised();
    let output = taste(dir.path(), &["targets", "--json", "--quiet"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert_eq!(text.lines().count(), 1);
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "marker_targets": ["CLAUDE.md", ".windsurfrules", "AGENTS.md", ".github/copilot-instructions.md"],
            "cursor_rules_dir": ".cursor/rules",
            "cursor_rule_prefix": "opentaste-",
            "cursor_rule_extension": "mdc"
        })
    );
}

#[test]
fn disable_and_enable_toggle_a_rule_and_resync() {
    let dir = initialised();
    for _ in 0..6 {
        learn_delta(
            dir.path(),
            "src/users/user.entity.ts",
            ENTITY_BEFORE,
            ENTITY_AFTER,
        );
    }
    let rules: serde_json::Value =
        serde_json::from_str(&read(dir.path(), ".taste/rules.json")).unwrap();
    let id = rules["rules"][0]["id"].as_str().unwrap().to_string();
    assert!(read(dir.path(), "CLAUDE.md").contains("@AutoEntity"));

    assert!(taste(dir.path(), &["disable", &id]).status.success());
    let rules: serde_json::Value =
        serde_json::from_str(&read(dir.path(), ".taste/rules.json")).unwrap();
    assert_eq!(rules["rules"][0]["enabled"], false);
    assert!(!std::fs::read_to_string(dir.path().join("CLAUDE.md"))
        .unwrap_or_default()
        .contains("@AutoEntity"));

    assert!(taste(dir.path(), &["enable", &id]).status.success());
    assert!(read(dir.path(), "CLAUDE.md").contains("@AutoEntity"));

    let missing = taste(dir.path(), &["disable", "arch-nope-000000"]);
    assert!(!missing.status.success());
    assert!(stderr(&missing).contains("arch-nope-000000"));
}
