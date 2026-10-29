//! `taste learn-delta`: learn from one edit the editor extension saw.
//!
//! This is a cross-phase contract, fixed in `plans/PHASE-5.md` step 3 and
//! consumed by phase 6. stdin carries `{"before": …, "after": …}`. stdout
//! carries exactly one line, `{"promoted": [TasteRule…], "candidates":
//! [{id, confidence, instruction}…]}`, both arrays always present, and
//! nothing else. Errors go to stderr with a non-zero exit and an empty stdout.
//!
//! Every observation recorded here is also remembered in the cache, so the
//! post-commit hook does not count the same correction again when it is
//! committed. See [`crate::commands::delta_key`].

use std::io::{Read, Write};
use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use taste_core::{Outcome, RuleStore, TasteRule};

use crate::commands::delta_key;

/// A remembered edit the hook never claimed is forgotten after this long.
const PENDING_DELTA_DAYS: i64 = 30;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Delta {
    before: String,
    after: String,
}

#[derive(Debug, Serialize)]
pub struct Learned {
    pub promoted: Vec<TasteRule>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Serialize)]
pub struct Candidate {
    pub id: String,
    pub confidence: f32,
    pub instruction: String,
}

pub fn run(store: &RuleStore, file: &Path) -> anyhow::Result<()> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .context("reading the edit from stdin")?;
    let delta: Delta = serde_json::from_str(&raw)
        .context("stdin must be one JSON object: {\"before\": \"…\", \"after\": \"…\"}")?;
    let learned = learn(store, file, &delta.before, &delta.after)?;
    // Encode fully before writing, so a failure cannot leave half a line.
    let line = serde_json::to_string(&learned)?;
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()?;
    Ok(())
}

/// Analyse one edit, record what it shows, and sync if a rule was confirmed.
pub fn learn(store: &RuleStore, file: &Path, before: &str, after: &str) -> anyhow::Result<Learned> {
    // The scope is derived from the path, so an absolute path would widen it
    // to `**/*.ext`.
    let relative = store.paths().relativize(file).to_path_buf();
    let observations = taste_core::analyze_in(store.paths(), before, after, &relative)?;
    store
        .candidates()
        .prune_deltas(chrono::Utc::now() - chrono::Duration::days(PENDING_DELTA_DAYS))?;
    let mut learned = Learned {
        promoted: Vec::new(),
        candidates: Vec::new(),
    };
    for observation in observations {
        let instruction = observation.instruction.clone();
        let key = delta_key(&relative, &observation);
        let outcome = store.record_observation(observation)?;
        store.candidates().remember_delta(&key)?;
        match outcome {
            Outcome::Promoted(rule) => learned.promoted.push(*rule),
            Outcome::Created { id, confidence } | Outcome::Reinforced { id, confidence, .. } => {
                learned.candidates.push(Candidate {
                    id,
                    confidence,
                    instruction,
                })
            }
            Outcome::AlreadyConfirmed { .. } | Outcome::Conflicts { .. } => {}
        }
    }
    if !learned.promoted.is_empty() {
        store.sync()?;
    }
    Ok(learned)
}
