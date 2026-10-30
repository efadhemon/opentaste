//! `taste watch`: rescan when source changes, resync when rules change.
//!
//! `notify` 8 ships no debouncer, so a burst of events is folded by a small
//! time window here. OpenTaste's own output is filtered explicitly using
//! `Paths::is_generated_target`, the same list the transpiler writes, rather
//! than relying on the transpiler skipping unchanged writes.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use notify::{RecursiveMode, Watcher};
use taste_core::config::Paths;
use taste_core::{RuleStore, SupportedLanguage};

use crate::commands::sync;
use crate::output::Output;

/// Quiet time after the last event before acting. FSEvents on macOS already
/// coalesces and delivers late; inotify on Linux delivers every write at once,
/// so a shorter window is enough there.
#[cfg(target_os = "macos")]
const WINDOW: Duration = Duration::from_millis(700);
#[cfg(not(target_os = "macos"))]
const WINDOW: Duration = Duration::from_millis(300);
/// Act at the latest this long after the first event, even if events keep
/// coming (a formatter rewriting a whole tree, say).
const MAX_DELAY: Duration = Duration::from_secs(5);
const TICK: Duration = Duration::from_millis(100);

/// Directories whose changes never matter, wherever they appear.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".taste",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    "vendor",
    ".next",
    ".turbo",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relevance {
    Ignore,
    /// A script file the scanner reads.
    Source,
    /// `.taste/rules.json`, edited by hand or by a `git pull`.
    Rules,
}

/// What a change to `relative` (a path relative to the project root) means.
///
/// `new_dir` marks a directory that was just created. It counts as a source
/// change in its own right: the watch on a new directory is only added once
/// its creation is reported, so files written into it straight away (a
/// `git checkout` adding a folder, say) can arrive before anyone is
/// listening. The rescan walks the whole tree, so it picks them up anyway.
pub fn relevance(paths: &Paths, relative: &Path, new_dir: bool) -> Relevance {
    if paths.project_root.join(relative) == paths.rules_json {
        return Relevance::Rules;
    }
    if paths.is_generated_target(relative) {
        return Relevance::Ignore;
    }
    let ignored = relative
        .components()
        .any(|part| IGNORED_DIRS.iter().any(|dir| part.as_os_str() == *dir));
    if ignored {
        return Relevance::Ignore;
    }
    if new_dir {
        // `.cursor/rules` and `.github` appear when a sync first writes into
        // them; that is our own output, not new source.
        let absolute = paths.project_root.join(relative);
        let holds_targets = absolute == paths.cursor_rules_dir()
            || paths.marker_targets().iter().any(|target| {
                target.parent() == Some(absolute.as_path()) && absolute != paths.project_root
            })
            || paths.cursor_rules_dir().parent() == Some(absolute.as_path());
        return if holds_targets {
            Relevance::Ignore
        } else {
            Relevance::Source
        };
    }
    match SupportedLanguage::from_path(relative) {
        Some(lang) if lang.has_analyzers() => Relevance::Source,
        _ => Relevance::Ignore,
    }
}

/// Folds a burst of events into one action.
#[derive(Debug, Default)]
pub struct Debouncer {
    first: Option<Instant>,
    last: Option<Instant>,
    pub source: bool,
    pub rules: bool,
}

impl Debouncer {
    pub fn note(&mut self, relevance: Relevance, now: Instant) {
        match relevance {
            Relevance::Ignore => return,
            Relevance::Source => self.source = true,
            Relevance::Rules => self.rules = true,
        }
        self.first.get_or_insert(now);
        self.last = Some(now);
    }

    /// Whether the pending work should run now.
    pub fn due(&self, now: Instant) -> bool {
        match (self.first, self.last) {
            (Some(first), Some(last)) => {
                now.duration_since(last) >= WINDOW || now.duration_since(first) >= MAX_DELAY
            }
            _ => false,
        }
    }

    pub fn reset(&mut self) {
        *self = Debouncer::default();
    }
}

pub fn run(store: &RuleStore, out: &Output) -> anyhow::Result<()> {
    let paths = store.paths().clone();
    // Events carry canonical paths (`/private/var/…` on macOS for `/var/…`).
    let canonical_root = paths
        .project_root
        .canonicalize()
        .with_context(|| format!("resolving {}", paths.project_root.display()))?;

    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .context("starting the file watcher")?;
    watcher
        .watch(&canonical_root, RecursiveMode::Recursive)
        .with_context(|| format!("watching {}", canonical_root.display()))?;
    out.say(format!(
        "Watching {} for source and rule changes. Press Ctrl-C to stop.",
        paths.project_root.display()
    ));

    let classify = |event: &notify::Event| -> Vec<Relevance> {
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return Vec::new();
        }
        let created = matches!(event.kind, notify::EventKind::Create(_));
        event
            .paths
            .iter()
            .filter_map(|path| {
                let relative = relative_to(path, &canonical_root, &paths.project_root)?;
                let new_dir = created && path.is_dir();
                Some(relevance(&paths, &relative, new_dir))
            })
            .collect()
    };

    let mut pending = Debouncer::default();
    loop {
        match receiver.recv_timeout(TICK) {
            Ok(Ok(event)) => {
                let now = Instant::now();
                for relevance in classify(&event) {
                    pending.note(relevance, now);
                }
            }
            Ok(Err(err)) => eprintln!("taste watch: {err}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if !pending.due(Instant::now()) {
            continue;
        }
        let (source, rules) = (pending.source, pending.rules);
        pending.reset();
        act(store, out, source, rules)?;

        // Our own run rewrote `.taste/rules.json` at most; drop those events
        // so the watcher does not answer itself, but keep any source edits
        // that landed meanwhile.
        while let Ok(Ok(event)) = receiver.recv_timeout(WINDOW) {
            let now = Instant::now();
            for relevance in classify(&event) {
                if relevance == Relevance::Source {
                    pending.note(relevance, now);
                }
            }
        }
    }
}

fn act(store: &RuleStore, out: &Output, source: bool, rules: bool) -> anyhow::Result<()> {
    let stamp = chrono::Local::now().format("%H:%M:%S");
    if source {
        out.say(format!("[{stamp}] source changed; rescanning"));
        let report = taste_core::scan(store)?;
        for outcome in &report.outcomes {
            out.say(format!("  {}", crate::commands::describe_outcome(outcome)));
        }
    } else if rules {
        out.say(format!("[{stamp}] .taste/rules.json changed; syncing"));
    }
    let report = store.sync()?;
    if !report.is_empty() {
        sync::report_sync(&report, store, out);
    }
    Ok(())
}

/// `path` relative to the project root, accepting either the canonical or
/// the configured spelling of the root.
fn relative_to(path: &Path, canonical_root: &Path, root: &Path) -> Option<PathBuf> {
    path.strip_prefix(canonical_root)
        .or_else(|_| path.strip_prefix(root))
        .ok()
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths::for_root("/proj")
    }

    #[test]
    fn source_rules_and_noise_are_told_apart() {
        let paths = paths();
        let rel = |path: &str| relevance(&paths, Path::new(path), false);
        assert_eq!(rel("src/app.ts"), Relevance::Source);
        assert_eq!(rel("src/App.jsx"), Relevance::Source);
        assert_eq!(rel(".taste/rules.json"), Relevance::Rules);
        assert_eq!(rel(".taste/cache.db"), Relevance::Ignore);
        assert_eq!(rel(".taste/cache.db-wal"), Relevance::Ignore);
        assert_eq!(rel("node_modules/pkg/index.ts"), Relevance::Ignore);
        assert_eq!(rel("packages/ui/dist/index.js"), Relevance::Ignore);
        assert_eq!(rel(".git/index"), Relevance::Ignore);
        assert_eq!(rel("README.md"), Relevance::Ignore);
        assert_eq!(rel("app/users.py"), Relevance::Source);
        assert_eq!(rel("src/lib.rs"), Relevance::Source);
        assert_eq!(rel("cmd/main.go"), Relevance::Source);
        assert_eq!(
            rel("src/Main.java"),
            Relevance::Ignore,
            "only languages with analyzers are scanned"
        );
    }

    #[test]
    fn every_transpiler_target_is_ignored() {
        let paths = paths();
        for target in paths.marker_targets() {
            let relative = target.strip_prefix(&paths.project_root).unwrap();
            assert_eq!(
                relevance(&paths, relative, false),
                Relevance::Ignore,
                "{relative:?}"
            );
        }
        assert_eq!(
            relevance(
                &paths,
                Path::new(".cursor/rules/opentaste-imports.mdc"),
                false
            ),
            Relevance::Ignore
        );
    }

    #[test]
    fn a_new_directory_counts_unless_it_is_one_we_ignore() {
        let paths = paths();
        assert_eq!(
            relevance(&paths, Path::new("src/feature"), true),
            Relevance::Source
        );
        assert_eq!(
            relevance(&paths, Path::new("src/feature"), false),
            Relevance::Ignore
        );
        assert_eq!(
            relevance(&paths, Path::new("node_modules/x"), true),
            Relevance::Ignore
        );
        assert_eq!(
            relevance(&paths, Path::new(".cursor/rules"), true),
            Relevance::Ignore
        );
        assert_eq!(
            relevance(&paths, Path::new(".cursor"), true),
            Relevance::Ignore
        );
        assert_eq!(
            relevance(&paths, Path::new(".github"), true),
            Relevance::Ignore
        );
    }

    #[test]
    fn a_burst_runs_once_after_the_window_closes() {
        let start = Instant::now();
        let mut debouncer = Debouncer::default();
        assert!(!debouncer.due(start), "nothing pending");
        debouncer.note(Relevance::Ignore, start);
        assert!(
            !debouncer.due(start + WINDOW * 2),
            "noise schedules nothing"
        );

        debouncer.note(Relevance::Source, start);
        debouncer.note(Relevance::Source, start + WINDOW / 2);
        assert!(
            !debouncer.due(start + WINDOW),
            "the second event extends the window"
        );
        assert!(debouncer.due(start + WINDOW / 2 + WINDOW));
        assert!(debouncer.source && !debouncer.rules);

        debouncer.reset();
        assert!(!debouncer.due(start + MAX_DELAY * 2));
    }

    #[test]
    fn a_steady_stream_still_runs_after_the_maximum_delay() {
        let start = Instant::now();
        let mut debouncer = Debouncer::default();
        let mut now = start;
        while now < start + MAX_DELAY {
            debouncer.note(Relevance::Rules, now);
            assert!(!debouncer.due(now), "{:?}", now - start);
            now += WINDOW / 3;
        }
        debouncer.note(Relevance::Rules, now);
        assert!(debouncer.due(now));
    }

    #[test]
    fn event_paths_are_relativised_against_either_spelling_of_the_root() {
        let canonical = Path::new("/private/var/proj");
        let root = Path::new("/var/proj");
        assert_eq!(
            relative_to(Path::new("/private/var/proj/src/a.ts"), canonical, root),
            Some(PathBuf::from("src/a.ts"))
        );
        assert_eq!(
            relative_to(Path::new("/var/proj/src/a.ts"), canonical, root),
            Some(PathBuf::from("src/a.ts"))
        );
        assert_eq!(
            relative_to(Path::new("/elsewhere/a.ts"), canonical, root),
            None
        );
    }
}
