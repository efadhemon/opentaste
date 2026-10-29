# Phase 4: Local stdio MCP server

**Status:** completed

| | |
| --- | --- |
| **Blocked by** | **Phase 0 (CI)** — part 0 of [PHASE-3.md](PHASE-3.md). Run it first even if you start here. No code dependency on anything unfinished. |
| **Blocks** | [Phase 5](PHASE-5.md) — the `taste mcp` subcommand is a thin wrapper over `serve()` |
| **Independent of** | [Phase 3](PHASE-3.md) — shares no code; either order works |
| **Rough size** | 1 session |

Small, self-contained, and buildable immediately: every call it needs already
exists and is tested. PLAN.md deliberately sequences this before phase 3 —
finishing it makes OpenTaste genuinely usable, because an editor can record and
retrieve preferences through chat well before the scanner exists. That is a far
better place to pause than halfway through the diff engine.

`crates/taste-mcp` currently contains only
`pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05"`.

---

## Part 0 — Dependencies

- [x] No new runtime dependencies. `taste-core`, `serde`, `serde_json` and
      `anyhow` are already declared in `crates/taste-mcp/Cargo.toml`.
- [x] Add a `[dev-dependencies]` section with `tempfile.workspace = true` and
      `pretty_assertions.workspace = true`.
- [x] Add `#![deny(clippy::print_stdout)]` at the top of `src/lib.rs`. See the
      stdout hazard in step 2 — this is the cheapest possible insurance against
      it.

---

## Step 1: `protocol.rs`

- [x] JSON-RPC 2.0 types: `Request`, `Response`, `ResponseError`, and a
      notification shape. `id` is `Option<Value>` and absent means notification.
- [x] Standard error codes as named constants: `-32700` parse error, `-32600`
      invalid request, `-32601` method not found, `-32602` invalid params,
      `-32603` internal error.
- [x] Constructors: `Response::ok(id, result)` and `Response::error(id, code, message)`.
- [x] `serde` attributes so a response never emits both `result` and `error`, and
      `"jsonrpc": "2.0"` is always present.

**Hazard: MCP's stdio transport is newline-delimited JSON, not
`Content-Length`-framed like the Language Server Protocol.** One JSON object per
line, no headers, no blank-line separator. Implementing LSP framing here is the
single most common way to end up with an MCP server that never completes a
handshake and gives you nothing to debug.

---

## Step 2: `server.rs` — the stdio loop

- [x] `pub fn serve<R: BufRead, W: Write>(reader: R, writer: W, store: &RuleStore) -> anyhow::Result<()>`.
      This crate is a transport adapter rather than part of the core engine, so
      `anyhow` is the right choice here and is already a declared dependency.
- [x] Handle `initialize`, the `notifications/initialized` acknowledgement,
      `tools/list`, `tools/call` and `ping`.
- [x] `initialize` result:
      `{ "protocolVersion": …, "capabilities": { "tools": {} }, "serverInfo": { "name": "opentaste", "version": env!("CARGO_PKG_VERSION") } }`.
      Echo the client's `protocolVersion` back when it is one you support, and
      fall back to `MCP_PROTOCOL_VERSION` otherwise. Supported:
      `SUPPORTED_PROTOCOL_VERSIONS` (`2024-11-05`, `2025-03-26`,
      `2025-06-18`); the tool result shape is the same in all three.
- [x] Notifications get **no response at all**. A response to a notification is a
      protocol violation that some clients treat as fatal.
- [x] `ping` returns an empty result object.
- [x] A line that fails to parse produces a `-32700` response and the loop
      **continues**. One bad line must never end the session. That includes a
      line that is not UTF-8, which is why the loop reads bytes with
      `read_until` rather than `lines()`. Valid JSON that is not a request
      (no `method`, wrong `jsonrpc`) is `-32600`, answered against its `id`.
- [x] An unknown method produces `-32601`.
- [x] Flush the writer after every response. A buffered reply that never leaves
      the process looks exactly like a hung server.
- [x] Add `crates/taste-mcp/examples/stdio.rs` — three lines wiring
      `Paths::discover`, `RuleStore::open` and `serve(stdin().lock(),
      stdout().lock(), &store)`. It makes the server runnable, and therefore
      testable against a real client, before [Phase 5](PHASE-5.md) adds
      `taste mcp`. It takes an optional project path as its first argument,
      because editors do not all start MCP servers in the project root.

**Key decision: make the loop generic over `BufRead` and `Write`** —
`serve(reader, writer, store)` rather than reading `std::io::stdin()` directly.
That is the whole difference between testing the protocol with string buffers in
milliseconds and having to spawn subprocesses.

**Hazard: stdout is the protocol channel.** A single stray `println!` corrupts
the stream and the client disconnects with no useful diagnostic. All logging goes
to stderr, which is why `clippy::print_stdout` is denied crate-wide in part 0.

---

## Step 3: `tools.rs` — the three tools

Per SPEC.md section 7, each mapping onto one call that already exists and is
already tested.

| Tool | Implementation |
| --- | --- |
| `get_coding_taste` | `RuleStore::rules_for_file(file_path)` |
| `record_coding_taste` | `record_observation(…, RuleSource::McpFeedback)` then `sync()` |
| `reject_coding_taste` | `set_enabled(id, false)`, or `delete_rule` when explicitly asked |

- [x] `tools/list` returns exactly these three, each with a `name`,
      `description` and a JSON Schema `inputSchema`.
- [x] The third tool is named **`reject_coding_taste`**. SPEC.md's directory
      listing in section 2 calls it `reject_taste` while section 7 calls it
      `reject_coding_taste`; section 7 defines the actual interface, so it wins.
      Spell it one way everywhere, because the name is part of the wire contract
      and renaming it later silently breaks every editor already configured.
- [x] Write `record_coding_taste`'s description as the instruction to the model it
      actually is: *"Call this tool immediately whenever the user corrects your
      code style, architecture, naming, or explicitly states a coding
      preference."* This string is the entire mechanism by which the loop gets
      triggered; a vague description means the tool is never called and the
      feature silently does not exist.
- [x] `record_coding_taste` input: `category` (required, one of the seven
      slugs), `scope` (required glob), `instruction` (required), `anti_pattern`
      and `preferred_pattern` (optional).
- [x] Validate `scope` with `storage::validate_scope` before recording. This is
      the one path where a glob arrives from outside the codebase.
- [x] Parse `category` through `RuleCategory::from_str`, surfacing
      `Error::InvalidEnumValue` as a tool error listing the valid slugs.
- [x] Every argument problem inside a known tool (missing, wrong type, bad
      glob, bad category) is a tool error with `isError: true`, so the model
      reads the message and retries. Only an unknown tool name is a protocol
      error (`-32602`).
- [x] An observation that is the opposite of a confirmed rule comes back from
      `record_observation` as `Outcome::Conflicts` (added in the phase 3
      review). `record_coding_taste` reports it as a tool error that names the
      existing rule and tells the model to call `reject_coding_taste` first.
- [x] `reject_coding_taste` input: `rule_id` (required) and `delete`
      (optional, default `false`).
- [x] After a successful `record_coding_taste`, call `RuleStore::sync()` and
      report what changed from the returned `SyncReport`, so the model can tell
      the user their preference reached their editor files.
- [x] Return results as `{ "content": [{ "type": "text", "text": … }], "isError": false }`.

Two details worth getting right:

- **Disable by default rather than delete.** The spec says "disables or
  deletes"; disabling is recoverable and deletion is not, so deletion is an
  explicit opt-in flag.
- **Tool failures are not protocol failures.** MCP distinguishes *tool
  execution* errors, returned inside the result with `isError: true`, from
  *protocol* errors, which are JSON-RPC error objects. A rule id that does not
  exist is the former. Getting this backwards makes a normal "no such rule"
  response look to the client like the server broke.

---

## Step 4: The full test set

**Every step above lands with its own tests and a clean
`cargo test --workspace`; see [AGENTS.md](../AGENTS.md).** This is the complete
list the phase cannot ship without, so you can confirm nothing was skipped — not
a batch of work to leave until last. The generic `serve` signature in step 2
exists so this is possible from the first handler onward.

Drive `serve()` with a scripted sequence of JSON lines over a `Cursor<Vec<u8>>`
and assert on the responses.

- [x] A full handshake: `initialize`, then `notifications/initialized`,
      asserting the notification produced no output line.
- [x] `tools/list` listing exactly three tools, with the names spelled as the
      spec spells them.
- [x] `record_coding_taste` landing a rule in `.taste/rules.json` with
      `confidence = 0.95` and `source = "mcp-feedback"`, and the editor targets
      written.
- [x] `get_coding_taste` returning only the rules whose scope matches the
      requested path.
- [x] `reject_coding_taste` disabling by default, and deleting only with
      `"delete": true`.
- [x] An unknown `rule_id` producing `isError: true` inside a *successful*
      response, not a JSON-RPC error.
- [x] An invalid `scope` glob rejected with a message naming the offending
      pattern.
- [x] A malformed line producing a `-32700` error and the loop surviving to
      answer the next valid request.
- [x] `ping` answered.

---

## Definition of done

- [x] `cargo fmt --all --check` clean.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [x] `cargo test --workspace` clean.
- [x] **Manual check:** a real handshake against Cursor with the server
      registered in `.cursor/mcp.json`, the three tools visible in the client,
      and a preference stated in chat landing in `.taste/rules.json`. Until
      [Phase 5](PHASE-5.md) ships `taste mcp`, register it with
      `cargo run -p taste-mcp --example stdio` or an equivalent temporary entry
      point. Done by hand in Cursor on 2026-10-29: the three tools were
      visible and a preference stated in chat landed in `.taste/rules.json`.
      Before that, as an automated stand-in: the
      built example, piped a real handshake from a fresh `git init` directory,
      answered `initialize`, stayed silent on `notifications/initialized`,
      recorded a rule into `.taste/rules.json` and wrote all five editor
      targets, then returned it from `get_coding_taste`. Suggested entry, after
      `cargo build -p taste-mcp --example stdio`:

      ```json
      {
        "mcpServers": {
          "opentaste": {
            "command": "/abs/path/to/opentaste/target/debug/examples/stdio",
            "args": ["/abs/path/to/the/project"]
          }
        }
      }
      ```
- [x] Status at the top of this file set to `completed`.
