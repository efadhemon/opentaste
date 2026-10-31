//! `taste global`: the developer's personal rules, across projects.
//!
//! Nothing is personal unless the developer says so here (or tells the AI
//! with `record_coding_taste`'s `"global": true`). `suggest` only lists rules
//! confirmed in two or more projects; `add` is what makes one personal.
//! Personal rules never reach a project's committed files.

use anyhow::{bail, Context};
use taste_core::config::Paths;
use taste_core::storage::GlobalStore;
use taste_core::{RuleStore, TasteRule};

use crate::commands::sync;
use crate::output::Output;

/// What `taste global` was asked to do.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Action {
    /// Make a rule personal, so it is offered in every project.
    Add {
        /// A confirmed rule's id here, or one `taste global suggest` lists.
        rule_id: String,
        /// Also keep its avoid/prefer snippets, which are this project's code.
        #[arg(long)]
        keep_snippets: bool,
    },
    /// List personal rules.
    List,
    /// Stop treating a rule as personal.
    Remove { rule_id: String },
    /// List rules confirmed in two or more projects that are not personal yet.
    Suggest,
    /// Stop serving personal rules in this project.
    Off,
    /// Serve personal rules in this project again.
    On,
}

pub fn run(store: &RuleStore, action: Action, out: &Output) -> anyhow::Result<()> {
    let global = match store.global() {
        Some(global) => global,
        None => {
            // Say why rather than "unavailable".
            let home = Paths::home()?;
            GlobalStore::open(&home)
                .with_context(|| format!("opening {}", home.join(".taste/global.db").display()))?;
            bail!("personal rules could not be opened");
        }
    };
    match action {
        Action::Add {
            rule_id,
            keep_snippets,
        } => {
            let rule = find(store, global, &rule_id)?;
            let new = global.add(&rule, keep_snippets, &store.paths().project_root)?;
            out.say(format!(
                "{} {}: {}",
                if new { "personal  " } else { "updated   " },
                rule.id,
                rule.instruction
            ));
            if !keep_snippets && (rule.anti_pattern.is_some() || rule.preferred_pattern.is_some()) {
                out.say(
                    "  Its snippets were left in this project; pass --keep-snippets to take them.",
                );
            }
            sync::report_sync(&store.sync()?, store, out);
        }
        Action::List => {
            let rules = global.list()?;
            if rules.is_empty() {
                out.say("No personal rules. `taste global suggest` lists candidates.");
            }
            for rule in rules {
                out.say(format!(
                    "  {}  {}  {}",
                    rule.id, rule.scope, rule.instruction
                ));
            }
        }
        Action::Remove { rule_id } => {
            if !global.remove(&rule_id)? {
                bail!("{rule_id} is not a personal rule");
            }
            out.say(format!("Removed {rule_id} from your personal rules."));
            sync::report_sync(&store.sync()?, store, out);
        }
        Action::Suggest => {
            let suggestions = global.suggestions()?;
            if suggestions.is_empty() {
                out.say(
                    "Nothing to suggest: no rule is confirmed in two or more of your projects yet.",
                );
            }
            for suggestion in suggestions {
                out.say(format!(
                    "  {} ({} projects): {}",
                    suggestion.rule.id,
                    suggestion.projects.len(),
                    suggestion.rule.instruction
                ));
            }
        }
        Action::Off => {
            store.set_global_off(true)?;
            out.say("Personal rules are no longer served in this project.");
        }
        Action::On => {
            store.set_global_off(false)?;
            out.say("Personal rules are served in this project again.");
        }
    }
    Ok(())
}

/// A confirmed rule of this project, or a suggested one, by id.
fn find(store: &RuleStore, global: &GlobalStore, id: &str) -> anyhow::Result<TasteRule> {
    if let Some(rule) = store.rules()?.find(id) {
        return Ok(rule.clone());
    }
    if let Some(suggestion) = global.suggestions()?.into_iter().find(|s| s.rule.id == id) {
        return Ok(suggestion.rule);
    }
    bail!("{id} is neither a confirmed rule here nor a suggestion; `taste status` lists rule ids")
}
