//! `taste scan`: infer rules from the repository's code.

use taste_core::{RuleStore, ScanReport};

use crate::commands::describe_outcome;
use crate::output::Output;

pub fn run(store: &RuleStore, out: &Output) -> anyhow::Result<()> {
    let report = taste_core::scan(store)?;
    report_scan(&report, out);
    Ok(())
}

pub fn report_scan(report: &ScanReport, out: &Output) {
    out.say(format!(
        "Scanned {} file{}, skipped {} (unchanged, too large or not UTF-8).",
        report.files_scanned,
        plural(report.files_scanned),
        report.files_skipped
    ));
    if report.outcomes.is_empty() && report.retracted.is_empty() {
        out.say("No new rules: no pattern cleared the 80% / 10-file floor.");
    }
    for outcome in &report.outcomes {
        out.say(format!("  {}", describe_outcome(outcome)));
    }
    for rule in &report.retracted {
        out.say(format!(
            "  retracted  {} ({}): its compiler setting is gone.",
            rule.id, rule.scope
        ));
    }
}

pub fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}
