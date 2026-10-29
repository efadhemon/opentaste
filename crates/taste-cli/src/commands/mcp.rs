//! `taste mcp`: the stdio MCP server, and nothing else.
//!
//! stdout is the protocol channel, so this command prints nothing of its own.

use taste_core::RuleStore;

pub fn run(store: &RuleStore) -> anyhow::Result<()> {
    taste_mcp::serve(std::io::stdin().lock(), std::io::stdout().lock(), store)
}
