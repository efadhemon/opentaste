//! `taste status`: what OpenTaste knows and whether the editors have it.

use taste_core::ast::stored_tally;
use taste_core::transpiler::check_all;
use taste_core::{CandidateRule, RuleStore, TasteRule};

use crate::commands::scan::plural;
use crate::output::{glyphs, Output};

pub fn run(store: &RuleStore, out: &Output) -> anyhow::Result<()> {
    let paths = store.paths();
    let rules = store.rules()?;
    let candidates = store.candidate_rules()?;
    let glyph = glyphs();

    out.say(format!(
        "OpenTaste status for {}",
        paths.project_root.display()
    ));
    let context = &rules.project_context;
    let stack = if context.detected_stack.is_empty() {
        "not detected".to_string()
    } else {
        context.detected_stack.join(", ")
    };
    out.say(format!(
        "Stack: {stack} · {} file{} in the last scan",
        context.total_files_scanned,
        plural(context.total_files_scanned)
    ));
    match crate::commands::history::summary(store)? {
        Some((commits, newest, at)) => out.say(format!(
            "History: {commits} commit{} read, up to {newest}, last on {}",
            plural(commits),
            at.get(..10).unwrap_or(&at)
        )),
        None => out.say("History: not read yet; `taste scan --history 200` reads it"),
    }

    out.say("");
    out.say(format!("Confirmed rules ({})", rules.rules.len()));
    if rules.rules.is_empty() {
        out.say("  none yet");
    }
    for rule in &rules.rules {
        out.say(confirmed_line(rule, &glyph));
    }

    out.say("");
    out.say(format!("Candidates ({})", candidates.len()));
    if candidates.is_empty() {
        out.say("  none");
    }
    for candidate in &candidates {
        out.say(candidate_line(candidate, &glyph));
    }

    if store.global().is_some() {
        out.say("");
        let personal = store.personal_rules_served()?;
        if store.global_off()? {
            out.say("Personal rules: off in this project (`taste global on` serves them again)");
        } else {
            out.say(format!("Personal rules served here ({})", personal.len()));
            if personal.is_empty() {
                out.say("  none; `taste global suggest` lists rules you could make personal");
            }
            for rule in &personal {
                out.say(format!(
                    "  (personal) {}  {}  {}",
                    rule.id, rule.scope, rule.instruction
                ));
            }
        }
    }

    out.say("");
    out.say("Scan counts (why a rule was or was not inferred)");
    let tally = stored_tally(store)?;
    let measured: Vec<_> = tally
        .dimensions()
        .into_iter()
        .filter(|dimension| dimension.count.files > 0)
        .collect();
    if measured.is_empty() {
        out.say("  nothing measured yet; run `taste scan`");
    }
    for dimension in measured {
        let count = dimension.count;
        out.say(format!(
            "  {:<6} {:<13} {} {} · {} {} · {} file{}",
            dimension.family.name(),
            dimension.name,
            dimension.left,
            count.left,
            dimension.right,
            count.right,
            count.files,
            plural(count.files as usize)
        ));
    }

    out.say("");
    out.say("Editor targets");
    let pending = check_all(paths, &rules.rules)?;
    let show = |path: &std::path::PathBuf| paths.relativize(path).display().to_string();
    for path in &pending.unchanged {
        out.say(format!("  up to date {}", show(path)));
    }
    for path in &pending.written {
        out.say(format!("  stale      {}", show(path)));
    }
    for path in &pending.removed {
        out.say(format!("  leftover   {}", show(path)));
    }
    if pending.touched().next().is_none() {
        out.say("  none yet: nothing is confirmed");
    } else if !pending.is_empty() {
        out.say("  run `taste sync` to bring these up to date");
    }
    Ok(())
}

fn confirmed_line(rule: &TasteRule, glyph: &crate::output::Glyphs) -> String {
    let mark = if !rule.enabled {
        glyph.off
    } else if rule.is_active() {
        glyph.confirmed
    } else {
        glyph.off
    };
    let state = if rule.enabled { "" } else { " (disabled)" };
    format!(
        "  {mark} {:>3.0}% {:<14} {:<22} {}{state}\n        id {} · seen {} time{} · {}",
        rule.confidence * 100.0,
        rule.category.slug(),
        rule.scope,
        rule.instruction,
        rule.id,
        rule.occurrences,
        plural(rule.occurrences as usize),
        rule.source.slug()
    )
}

fn candidate_line(candidate: &CandidateRule, glyph: &crate::output::Glyphs) -> String {
    format!(
        "  {} {:>3.0}% {:<14} {:<22} {}\n        id {} · seen {} time{}",
        glyph.candidate,
        candidate.confidence * 100.0,
        candidate.category.slug(),
        candidate.scope,
        candidate.instruction,
        candidate.id,
        candidate.occurrences,
        plural(candidate.occurrences as usize)
    )
}
