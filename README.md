# OpenTaste (`taste`)

The 100% local-first, zero-cloud universal "Taste Mode" engine for AI coding
editors (Cursor, Claude Code, Windsurf, VS Code Copilot, Zed).

OpenTaste passively learns a project's coding style, architectural patterns and
conventions, then compiles the high-confidence ones into whichever instruction
files your editor already reads. Everything runs locally: no telemetry, no
external server, no cloud LLM.

See [SPEC.md](SPEC.md) for the full architecture specification.

## Status

Under construction. Implemented so far:

- `crates/taste-core` — data models, path config, JSON + SQLite storage behind
  the `RuleStore` facade, confidence scoring and deduplication, and the
  universal transpiler for Cursor, Claude, Windsurf and Copilot/Agents targets.

Not yet implemented: the tree-sitter AST scanner and structural diff engine, the
stdio MCP server, the CLI and terminal dashboard, and the editor extension.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

MIT
