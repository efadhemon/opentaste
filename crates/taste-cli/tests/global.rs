//! `taste global` against the real binary, each test with its own home.

use std::path::Path;
use std::process::{Command, Output};

fn taste(home: &Path, dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taste"))
        .env("TASTE_HOME", home)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run taste")
}

fn ok(home: &Path, dir: &Path, args: &[&str]) -> String {
    let output = taste(home, dir, args);
    assert!(
        output.status.success(),
        "taste {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A project whose code confirms the path-alias rule on `taste init`.
fn aliased_project(home: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..10 {
        let path = dir.path().join(format!("src/a{index}.ts"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            "import { a } from \"@/lib/a\";\nexport const x = a;\n",
        )
        .unwrap();
    }
    ok(home, dir.path(), &["init", "--quiet"]);
    dir
}

fn alias_rule_id(dir: &Path) -> String {
    let rules: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(".taste/rules.json")).unwrap())
            .unwrap();
    rules["rules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rule| {
            rule["instruction"]
                .as_str()
                .unwrap()
                .contains("path aliases")
        })
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn a_rule_confirmed_in_two_projects_is_suggested_then_made_personal_and_removed() {
    let home = tempfile::tempdir().unwrap();
    let first = aliased_project(home.path());
    let none = ok(home.path(), first.path(), &["global", "suggest"]);
    assert!(none.contains("Nothing to suggest"), "{none}");

    let second = aliased_project(home.path());
    let id = alias_rule_id(second.path());
    let suggested = ok(home.path(), second.path(), &["global", "suggest"]);
    assert!(
        suggested.contains(&id) && suggested.contains("2 projects"),
        "{suggested}"
    );

    let added = ok(home.path(), second.path(), &["global", "add", &id]);
    assert!(added.contains("personal"), "{added}");
    let listed = ok(home.path(), second.path(), &["global", "list"]);
    assert!(listed.contains(&id), "{listed}");
    assert!(!ok(home.path(), second.path(), &["global", "suggest"]).contains(&id));

    ok(home.path(), second.path(), &["global", "remove", &id]);
    assert!(ok(home.path(), second.path(), &["global", "list"]).contains("No personal rules"));
    let again = taste(home.path(), second.path(), &["global", "remove", &id]);
    assert!(!again.status.success(), "removing twice is an error");
}

#[test]
fn an_unknown_id_is_refused_and_a_project_can_opt_out_and_back_in() {
    let home = tempfile::tempdir().unwrap();
    let project = aliased_project(home.path());
    let refused = taste(
        home.path(),
        project.path(),
        &["global", "add", "nope-000000"],
    );
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("neither a confirmed rule"));

    assert!(ok(home.path(), project.path(), &["global", "off"]).contains("no longer served"));
    assert!(
        ok(home.path(), project.path(), &["global", "on"]).contains("served in this project again")
    );
}

#[test]
fn personal_rules_live_under_taste_home_only() {
    let home = tempfile::tempdir().unwrap();
    let project = aliased_project(home.path());
    let id = alias_rule_id(project.path());
    ok(home.path(), project.path(), &["global", "add", &id]);
    assert!(home.path().join(".taste/global.db").is_file());
    assert!(!project.path().join(".taste/global.db").exists());
}

#[test]
fn status_lists_the_personal_rules_served_here() {
    let home = tempfile::tempdir().unwrap();
    let first = aliased_project(home.path());
    let id = alias_rule_id(first.path());
    ok(home.path(), first.path(), &["global", "add", &id]);

    let other = tempfile::tempdir().unwrap();
    ok(home.path(), other.path(), &["init", "--quiet"]);
    let status = ok(home.path(), other.path(), &["status"]);
    assert!(
        status.contains("Personal rules served here (1)"),
        "{status}"
    );
    assert!(status.contains(&format!("(personal) {id}")), "{status}");

    ok(home.path(), other.path(), &["global", "off"]);
    let off = ok(home.path(), other.path(), &["status"]);
    assert!(off.contains("Personal rules: off in this project"), "{off}");
}
