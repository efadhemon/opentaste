//! The `taste` binary.
//!
//! The project is resolved and the [`RuleStore`] opened once, here, and every
//! command borrows it. `init` is the exception, because it runs before a
//! project exists. Core errors become messages at this boundary.

mod commands;
mod git;
mod output;
mod tui;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use taste_core::config::Paths;
use taste_core::RuleStore;

use crate::output::Output;

#[derive(Debug, Parser)]
#[command(
    name = "taste",
    version,
    about = "Learn a project's coding taste and teach it to AI editors"
)]
struct Cli {
    /// Print nothing but errors. The git hook and the editor extension run
    /// `taste` non-interactively.
    #[arg(long, short, global = true)]
    quiet: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Set up OpenTaste in this project: `.taste/`, `.gitignore`, the Cursor
    /// MCP entry and the post-commit hook, then scan and sync.
    Init,
    /// Infer rules from the repository's code.
    Scan {
        /// Also learn from the last COMMITS non-merge commits on this branch,
        /// after the newest one already read. `0` reads none.
        #[arg(long, value_name = "COMMITS")]
        history: Option<usize>,
    },
    /// Write confirmed rules into every editor instruction file.
    Sync,
    /// Learn from one edit: reads `{"before": …, "after": …}` on stdin and
    /// prints the rules it promoted as one line of JSON.
    LearnDelta {
        /// The edited file, relative to the project root.
        #[arg(long)]
        file: PathBuf,
    },
    /// Learn from the last commit. Installed as the post-commit hook.
    Learn {
        /// Diff `HEAD` against its parent.
        #[arg(long, required = true)]
        from_git_head: bool,
    },
    /// Summarise rules, candidates, scan counts and editor targets.
    Status,
    /// Serve the Model Context Protocol on stdin and stdout.
    Mcp,
    /// Open the terminal dashboard.
    Ui,
    /// Rescan and resync whenever source files or `.taste/rules.json` change.
    Watch,
    /// List the files `taste sync` writes, which watchers must ignore.
    Targets {
        /// Print one line of JSON, for the editor extension.
        #[arg(long)]
        json: bool,
    },
    /// Stop applying a confirmed rule without deleting it, then sync.
    Disable {
        /// The rule's id, as `taste status` shows it.
        rule_id: String,
    },
    /// Apply a disabled rule again, then sync.
    Enable {
        /// The rule's id, as `taste status` shows it.
        rule_id: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let out = Output::new(cli.quiet);
    match run(cli.command, &out) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("taste: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command, out: &Output) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("reading the current directory")?;
    if let Command::Init = command {
        return commands::init::run(&cwd, out);
    }
    let store = open_store(&cwd)?;
    match command {
        Command::Init => unreachable!("handled above"),
        Command::Scan { history } => commands::scan::run(&store, history, out),
        Command::Sync => commands::sync::run(&store, out),
        Command::LearnDelta { file } => commands::learn_delta::run(&store, &file),
        Command::Learn { .. } => commands::learn::run(&store, out),
        Command::Status => commands::status::run(&store, out),
        Command::Mcp => commands::mcp::run(&store),
        Command::Ui => tui::run(&store),
        Command::Watch => commands::watch::run(&store, out),
        Command::Targets { json } => commands::targets::run(&store, json, out),
        Command::Disable { rule_id } => commands::toggle::run(&store, &rule_id, false, out),
        Command::Enable { rule_id } => commands::toggle::run(&store, &rule_id, true, out),
    }
}

/// The project around `cwd`, which must already have been initialised.
///
/// `Paths::discover` also accepts a bare git repository, but opening a store
/// there would create `.taste/` as a side effect of `taste status`.
fn open_store(cwd: &std::path::Path) -> anyhow::Result<RuleStore> {
    let paths = Paths::discover(cwd)?;
    if !paths.taste_dir.is_dir() {
        bail!(
            "OpenTaste is not set up in {}; run `taste init` first",
            paths.project_root.display()
        );
    }
    Ok(RuleStore::open(paths)?)
}
