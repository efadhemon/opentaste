//! `taste sync`: compile confirmed rules into every editor target.

use taste_core::transpiler::SyncReport;
use taste_core::RuleStore;

use crate::output::Output;

pub fn run(store: &RuleStore, out: &Output) -> anyhow::Result<()> {
    let report = store.sync()?;
    report_sync(&report, store, out);
    Ok(())
}

pub fn report_sync(report: &SyncReport, store: &RuleStore, out: &Output) {
    let paths = store.paths();
    let show = |path: &std::path::PathBuf| paths.relativize(path).display().to_string();
    for path in &report.written {
        out.say(format!("  updated    {}", show(path)));
    }
    for path in &report.removed {
        out.say(format!("  removed    {}", show(path)));
    }
    for path in &report.unchanged {
        out.say(format!("  up to date {}", show(path)));
    }
    if report.is_empty() {
        out.say("Editor files were already up to date.");
    }
}
