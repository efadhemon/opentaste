//! `taste disable` and `taste enable`: what the editor toast's [Ignore] runs.

use anyhow::bail;
use taste_core::RuleStore;

use crate::commands::sync;
use crate::output::Output;

pub fn run(store: &RuleStore, rule_id: &str, enable: bool, out: &Output) -> anyhow::Result<()> {
    if !store.set_enabled(rule_id, enable)? {
        bail!("no confirmed rule has id {rule_id:?}; `taste status` lists them");
    }
    out.say(format!(
        "{} {rule_id}.",
        if enable { "Enabled" } else { "Disabled" }
    ));
    let report = store.sync()?;
    sync::report_sync(&report, store, out);
    Ok(())
}
