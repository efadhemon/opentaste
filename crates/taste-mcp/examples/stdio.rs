//! Run the MCP server on stdin/stdout. A stand-in for `taste mcp` until phase
//! 5 ships the CLI.
//!
//! The project is found from the first argument, or from the current
//! directory when there is none. Editors do not all start MCP servers in the
//! project root, so pass the path explicitly when registering it.

use std::path::PathBuf;

use taste_core::{config::Paths, RuleStore};

fn main() -> anyhow::Result<()> {
    let start = match std::env::args_os().nth(1) {
        Some(path) => PathBuf::from(path),
        None => std::env::current_dir()?,
    };
    let store = RuleStore::open(Paths::discover(start)?)?;
    taste_mcp::serve(std::io::stdin().lock(), std::io::stdout().lock(), &store)
}
