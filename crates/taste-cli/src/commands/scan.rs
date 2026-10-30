//! `taste scan`: infer rules from the repository's code.

use taste_core::{RuleStore, ScanReport};

use crate::commands::history::{self, HistoryReport, Window};
use crate::commands::{describe_outcome, sync};
use crate::output::Output;

pub fn run(store: &RuleStore, history: Option<usize>, out: &Output) -> anyhow::Result<()> {
    let report = taste_core::scan(store)?;
    report_scan(&report, out);
    let mined = match history {
        Some(commits) => Some(read_history(store, Window::commits(commits), out)?),
        None => None,
    };
    // Promotion triggers a sync (SPEC.md §5.1), as it does for every other
    // learner; a retracted rule must leave the editor files too.
    let changed = report
        .outcomes
        .iter()
        .any(|outcome| outcome.promoted().is_some())
        || !report.retracted.is_empty()
        || mined.as_ref().is_some_and(HistoryReport::promoted);
    if changed {
        let synced = store.sync()?;
        sync::report_sync(&synced, store, out);
    }
    Ok(())
}

/// Learn from recent commits in `window`, printing progress and a summary.
pub fn read_history(
    store: &RuleStore,
    window: Window,
    out: &Output,
) -> anyhow::Result<HistoryReport> {
    let mut last = 0;
    let report = history::mine(store, window, &mut |done, total| {
        if done == total || done >= last + 25 {
            last = done;
            out.say(format!("Reading history: {done}/{total} commits"));
        }
    })?;
    report_history(&report, out);
    Ok(report)
}

pub fn report_history(report: &HistoryReport, out: &Output) {
    let skipped = report.skipped_bots + report.skipped_reverts + report.skipped_sweeps;
    if report.commits == 0 && skipped == 0 {
        return;
    }
    out.say(format!(
        "Read {} commit{} of history ({} skipped: {} by bots, {} reverts, {} touching over {} files); {} correction{} came from history.",
        report.commits,
        plural(report.commits),
        skipped,
        report.skipped_bots,
        report.skipped_reverts,
        report.skipped_sweeps,
        history::MAX_FILES,
        report.outcomes.len(),
        plural(report.outcomes.len()),
    ));
    for outcome in &report.outcomes {
        out.say(format!("  {}", describe_outcome(outcome)));
    }
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
