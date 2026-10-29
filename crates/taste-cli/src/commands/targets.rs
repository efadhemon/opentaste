//! `taste targets`: the files `taste sync` writes.
//!
//! The editor extension must ignore exactly these, and reads them from here
//! rather than keeping its own copy of the list. `--json` is one line:
//! `{"marker_targets": [..], "cursor_rules_dir": "..", "cursor_rule_prefix":
//! "..", "cursor_rule_extension": ".."}`, every path relative to the project
//! root with forward slashes.

use serde::Serialize;
use taste_core::config::{CURSOR_RULE_EXTENSION, CURSOR_RULE_PREFIX};
use taste_core::RuleStore;

use crate::output::Output;

#[derive(Debug, Serialize)]
struct Targets {
    marker_targets: Vec<String>,
    cursor_rules_dir: String,
    cursor_rule_prefix: &'static str,
    cursor_rule_extension: &'static str,
}

pub fn run(store: &RuleStore, json: bool, out: &Output) -> anyhow::Result<()> {
    let paths = store.paths();
    let relative =
        |path: &std::path::Path| paths.relativize(path).to_string_lossy().replace('\\', "/");
    let targets = Targets {
        marker_targets: paths.marker_targets().iter().map(|p| relative(p)).collect(),
        cursor_rules_dir: relative(&paths.cursor_rules_dir()),
        cursor_rule_prefix: CURSOR_RULE_PREFIX,
        cursor_rule_extension: CURSOR_RULE_EXTENSION,
    };
    if json {
        // Machine-readable: printed even under `--quiet`.
        println!("{}", serde_json::to_string(&targets)?);
        return Ok(());
    }
    for target in &targets.marker_targets {
        out.say(target);
    }
    out.say(format!(
        "{}/{}<category>.{}",
        targets.cursor_rules_dir, targets.cursor_rule_prefix, targets.cursor_rule_extension
    ));
    Ok(())
}
