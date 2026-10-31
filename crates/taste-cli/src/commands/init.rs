//! `taste init`: the command that touches the most developer-owned files.
//!
//! Every step follows the rule `transpiler/marker.rs` set: read what is there,
//! change only what OpenTaste owns, write atomically, and when the existing
//! file cannot be understood, leave it alone and say what to add by hand. Each
//! step is idempotent, so running `init` again reports instead of resetting.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::{json, Value};
use taste_core::config::Paths;
use taste_core::storage::atomic::{read_to_string_opt, write_atomic};
use taste_core::{Error, RuleStore};

use crate::commands::{scan, sync};
use crate::git;
use crate::output::Output;

/// What one setup step did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Created or changed a file.
    Changed(String),
    /// Already in place; nothing written.
    Done(String),
    /// Not applicable here, such as the hook outside a git repository.
    Skipped(String),
    /// Left alone because it could not be changed safely; the text says what
    /// to do by hand.
    Manual(String),
}

/// Lines `.gitignore` must hold. The database's WAL side files and the lock
/// are as local as the database itself.
pub const GITIGNORE_ENTRIES: &[&str] = &[
    ".taste/cache.db",
    ".taste/cache.db-wal",
    ".taste/cache.db-shm",
    ".taste/.lock",
];
const GITIGNORE_HEADER: &str = "# OpenTaste local state";

pub const HOOK_START: &str = "# >>> opentaste >>>";
pub const HOOK_END: &str = "# <<< opentaste <<<";
/// The command the hook runs. Backgrounded, silent, and skipped when `taste`
/// is not on the PATH, so a commit is never slowed down or failed by it.
pub const HOOK_LINE: &str =
    "command -v taste >/dev/null 2>&1 && (taste learn --from-git-head --quiet >/dev/null 2>&1 &)";

pub fn run(cwd: &Path, out: &Output) -> anyhow::Result<()> {
    let paths = match Paths::discover(cwd) {
        Ok(paths) => paths,
        Err(Error::ProjectRootNotFound { .. }) => Paths::for_root(cwd),
        Err(err) => return Err(err.into()),
    };
    let root = paths.project_root.clone();
    let fresh = !paths.rules_json.exists();

    let store = RuleStore::open(paths.clone()).context("creating .taste/cache.db")?;
    store.ensure_rules_file()?;
    if fresh {
        out.say(format!("Setting up OpenTaste in {}", root.display()));
        out.say("  created    .taste/rules.json and .taste/cache.db");
    } else {
        out.say(format!(
            "OpenTaste is already set up in {}; checking each step.",
            root.display()
        ));
    }

    let steps = [
        (".gitignore", ensure_gitignore(&root)?),
        (".cursor/mcp.json", ensure_mcp_json(&root)?),
        ("post-commit hook", ensure_hook(&root)?),
    ];
    for (name, step) in &steps {
        report(out, name, step);
    }

    let scanned = taste_core::scan(&store)?;
    scan::report_scan(&scanned, out);
    let mined = scan::read_history(
        &store,
        crate::commands::history::Window::INIT,
        &scanned.tallies,
        out,
    )?;
    let synced = store.sync()?;
    sync::report_sync(&synced, &store, out);

    let untouched = steps
        .iter()
        .all(|(_, step)| !matches!(step, Step::Changed(_)))
        && scanned.outcomes.is_empty()
        && scanned.retracted.is_empty()
        && mined.outcomes.is_empty()
        && synced.is_empty();
    if !fresh && untouched {
        out.say("Nothing to do: this project was already fully set up.");
    }
    Ok(())
}

fn report(out: &Output, name: &str, step: &Step) {
    let line = match step {
        Step::Changed(what) => format!("  updated    {name}: {what}"),
        Step::Done(what) => format!("  ok         {name}: {what}"),
        Step::Skipped(why) => format!("  skipped    {name}: {why}"),
        Step::Manual(what) => format!("  ACTION     {name}: {what}"),
    };
    out.say(line);
}

/// Add any missing OpenTaste entry to `.gitignore`, matching on each entry's
/// text so a reordered file never gets a second copy.
pub fn ensure_gitignore(root: &Path) -> anyhow::Result<Step> {
    let path = root.join(".gitignore");
    let existing = read_to_string_opt(&path)?.unwrap_or_default();
    let present: Vec<&str> = existing.lines().map(str::trim).collect();
    let covers = |entry: &str| {
        let anchored = format!("/{entry}");
        present.iter().any(|line| {
            *line == entry
                || *line == anchored
                || (entry.starts_with(".taste/cache.db") && *line == ".taste/cache.db*")
        })
    };
    let missing: Vec<&str> = GITIGNORE_ENTRIES
        .iter()
        .copied()
        .filter(|entry| !covers(entry))
        .collect();
    if missing.is_empty() {
        return Ok(Step::Done("already ignores the local cache".to_string()));
    }

    let newline = if existing.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut updated = existing.clone();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push_str(newline);
    }
    if !updated.is_empty() {
        updated.push_str(newline);
    }
    updated.push_str(GITIGNORE_HEADER);
    updated.push_str(newline);
    for entry in &missing {
        updated.push_str(entry);
        updated.push_str(newline);
    }
    write_atomic(&path, updated.as_bytes())?;
    Ok(Step::Changed(format!("added {}", missing.join(", "))))
}

fn opentaste_server() -> Value {
    json!({ "command": "taste", "args": ["mcp"] })
}

/// Register the MCP server in `.cursor/mcp.json`, merging into whatever is
/// already there. Every other key and server is kept, in its original order.
pub fn ensure_mcp_json(root: &Path) -> anyhow::Result<Step> {
    let path = root.join(".cursor").join("mcp.json");
    let by_hand = format!(
        "add this under \"mcpServers\" yourself: \"opentaste\": {}",
        opentaste_server()
    );
    let existing = read_to_string_opt(&path)?;
    let mut document: Value = match existing.as_deref() {
        None => json!({}),
        Some(raw) if raw.trim().is_empty() => json!({}),
        Some(raw) => match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(err) => {
                return Ok(Step::Manual(format!(
                    "left untouched because it is not valid JSON ({err}); {by_hand}"
                )))
            }
        },
    };
    let Some(top) = document.as_object_mut() else {
        return Ok(Step::Manual(format!(
            "left untouched because it is not a JSON object; {by_hand}"
        )));
    };
    let servers = top
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Ok(Step::Manual(format!(
            "left untouched because \"mcpServers\" is not an object; {by_hand}"
        )));
    };
    if let Some(current) = servers.get("opentaste") {
        return Ok(Step::Done(if *current == opentaste_server() {
            "already registers the opentaste server".to_string()
        } else {
            "kept your existing opentaste entry as it is".to_string()
        }));
    }
    servers.insert("opentaste".to_string(), opentaste_server());

    let indent = existing
        .as_deref()
        .map(detect_indent)
        .unwrap_or_else(|| "  ".to_string());
    let mut rendered = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut serializer = serde_json::Serializer::with_formatter(&mut rendered, formatter);
    serde::Serialize::serialize(&document, &mut serializer)?;
    rendered.push(b'\n');
    write_atomic(&path, &rendered)?;
    Ok(Step::Changed(if existing.is_some() {
        "added the opentaste server beside your others".to_string()
    } else {
        "created it with the opentaste server".to_string()
    }))
}

/// The indentation of the first indented line, so a rewrite keeps the
/// developer's style. Two spaces when nothing is indented.
fn detect_indent(raw: &str) -> String {
    raw.lines()
        .map(|line| {
            line.chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect::<String>()
        })
        .find(|indent| !indent.is_empty())
        .unwrap_or_else(|| "  ".to_string())
}

fn hook_block() -> String {
    format!(
        "{HOOK_START}\n# Learn from each commit in the background. Managed by `taste init`.\n{HOOK_LINE}\n{HOOK_END}\n"
    )
}

/// Install the post-commit hook without overwriting an existing one.
pub fn ensure_hook(root: &Path) -> anyhow::Result<Step> {
    if !git::is_repository(root) {
        return Ok(Step::Skipped(
            "not a git repository; run `taste init` again after `git init`".to_string(),
        ));
    }
    let by_hand = format!("add this line to your post-commit hook: {HOOK_LINE}");
    if let Some(custom) = git::text(root, &["config", "--get", "core.hooksPath"]) {
        if !custom.is_empty() {
            return Ok(Step::Manual(format!(
                "core.hooksPath is set to {custom}, which a hook manager owns; {by_hand}"
            )));
        }
    }
    let Some(dir) = git::text(root, &["rev-parse", "--git-path", "hooks"]) else {
        return Ok(Step::Manual(format!(
            "could not locate the hooks directory; {by_hand}"
        )));
    };
    let path: PathBuf = root.join(dir).join("post-commit");
    let block = hook_block();

    let Some(existing) = read_to_string_opt(&path)? else {
        write_atomic(&path, format!("#!/bin/sh\n{block}").as_bytes())?;
        make_executable(&path)?;
        return Ok(Step::Changed("installed".to_string()));
    };

    match (existing.find(HOOK_START), existing.find(HOOK_END)) {
        (Some(start), Some(end)) if start < end => {
            let end = end + HOOK_END.len();
            let end = if existing[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            if existing[start..end] == block {
                return Ok(Step::Done("already installed".to_string()));
            }
            let updated = format!("{}{block}{}", &existing[..start], &existing[end..]);
            write_atomic(&path, updated.as_bytes())?;
            Ok(Step::Changed("refreshed the OpenTaste section".to_string()))
        }
        (None, None) => {
            if let Some(reason) = cannot_append(&existing) {
                return Ok(Step::Manual(format!(
                    "left your hook untouched because {reason}; {by_hand}"
                )));
            }
            let mut updated = existing.clone();
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push('\n');
            updated.push_str(&block);
            write_atomic(&path, updated.as_bytes())?;
            make_executable(&path)?;
            Ok(Step::Changed("appended to your existing hook".to_string()))
        }
        _ => Ok(Step::Manual(format!(
            "left your hook untouched because its OpenTaste markers are unbalanced; {by_hand}"
        ))),
    }
}

/// Why shell lines appended to `hook` might never run, if they would not.
fn cannot_append(hook: &str) -> Option<&'static str> {
    let first = hook.lines().next().unwrap_or("");
    let shell = first.starts_with("#!")
        && ["sh", "bash", "zsh", "dash", "ksh"]
            .iter()
            .any(|name| first.split(['/', ' ']).any(|part| part == *name));
    if !shell {
        return Some("it is not a shell script");
    }
    if hook
        .lines()
        .any(|line| line.starts_with("exit") || line.starts_with("exec "))
    {
        return Some("it exits before reaching the end");
    }
    None
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn gitignore_gains_each_entry_once_even_when_reordered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        std::fs::write(&path, "node_modules/\n.taste/.lock\n").unwrap();
        assert!(matches!(
            ensure_gitignore(dir.path()).unwrap(),
            Step::Changed(_)
        ));
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(
            first.starts_with("node_modules/\n.taste/.lock\n"),
            "{first}"
        );
        for entry in GITIGNORE_ENTRIES {
            assert_eq!(
                first.lines().filter(|line| line == entry).count(),
                1,
                "{entry}"
            );
        }

        // Reorder by hand, then run again.
        let reordered: String = first
            .lines()
            .rev()
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(&path, &reordered).unwrap();
        assert!(matches!(
            ensure_gitignore(dir.path()).unwrap(),
            Step::Done(_)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), reordered);
    }

    #[test]
    fn a_wildcard_entry_counts_for_the_database_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(".gitignore"),
            ".taste/cache.db*\n/.taste/.lock\n",
        )
        .unwrap();
        assert!(matches!(
            ensure_gitignore(dir.path()).unwrap(),
            Step::Done(_)
        ));
    }

    #[test]
    fn crlf_gitignores_stay_crlf() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "dist/\r\n").unwrap();
        ensure_gitignore(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(!text.replace("\r\n", "").contains('\n'), "{text:?}");
    }

    #[test]
    fn mcp_json_keeps_other_servers_keys_order_and_indent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = "{\n    \"zeta\": 1,\n    \"mcpServers\": {\n        \"github\": {\n            \"command\": \"gh-mcp\"\n        }\n    }\n}\n";
        std::fs::write(&path, original).unwrap();

        assert!(matches!(
            ensure_mcp_json(dir.path()).unwrap(),
            Step::Changed(_)
        ));
        let written = std::fs::read_to_string(&path).unwrap();
        let value: Value = serde_json::from_str(&written).unwrap();
        assert_eq!(
            value["mcpServers"]["github"],
            json!({ "command": "gh-mcp" })
        );
        assert_eq!(value["mcpServers"]["opentaste"], opentaste_server());
        assert_eq!(value["zeta"], json!(1));
        assert!(
            written.find("zeta").unwrap() < written.find("mcpServers").unwrap(),
            "{written}"
        );
        assert!(written.contains("\n    \"zeta\""), "indent kept: {written}");

        assert!(matches!(
            ensure_mcp_json(dir.path()).unwrap(),
            Step::Done(_)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
    }

    #[test]
    fn a_customised_opentaste_entry_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"{"mcpServers":{"opentaste":{"command":"/opt/taste","args":["mcp"]}}}"#;
        std::fs::write(&path, original).unwrap();
        assert!(matches!(
            ensure_mcp_json(dir.path()).unwrap(),
            Step::Done(_)
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn invalid_or_unexpected_mcp_json_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".cursor/mcp.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        for original in ["{ \"mcpServers\": { ,", "[1, 2]", "{\"mcpServers\": 3}"] {
            std::fs::write(&path, original).unwrap();
            let step = ensure_mcp_json(dir.path()).unwrap();
            let Step::Manual(message) = step else {
                panic!("{original}: {step:?}");
            };
            assert!(message.contains("\"opentaste\""), "{message}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn a_hook_that_cannot_run_appended_lines_is_refused() {
        assert_eq!(cannot_append("#!/bin/sh\necho hi\n"), None);
        assert_eq!(cannot_append("#!/usr/bin/env bash\nset -e\n"), None);
        assert!(cannot_append("#!/usr/bin/env node\nconsole.log(1)\n").is_some());
        assert!(cannot_append("#!/bin/sh\nnpx lint\nexit 0\n").is_some());
        assert!(cannot_append("#!/bin/sh\nexec ./other-hook \"$@\"\n").is_some());
        assert!(cannot_append("echo no shebang\n").is_some());
    }
}
