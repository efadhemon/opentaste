# Phase 4: Local stdio MCP server

**Status:** pending

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

- [ ] No new runtime dependencies. `taste-core`, `serde`, `serde_json` and
      `anyhow` are already declared in `crates/taste-mcp/Cargo.toml`.
- [ ] Add a `[dev-dependencies]` section with `tempfile.workspace = true` and
      `pretty_assertions.workspace = true`.
- [ ] Add `#![deny(clippy::print_stdout)]` at the top of `src/lib.rs`. See the
      stdout hazard in step 2 — this is the cheapest possible insurance against
      it.

---

## Step 1: `protocol.rs`

- [ ] JSON-RPC 2.0 types: `Request`, `Response`, `ResponseError`, and a
      notification shape. `id` is `Option<Value>` and absent means notification.
- [ ] Standard error codes as named constants: `-32700` parse error, `-32600`
      invalid request, `-32601` method not found, `-32602` invalid params,
      `-32603` internal error.
- [ ] Constructors: `Response::ok(id, result)` and `Response::error(id, code, message)`.
- [ ] `serde` attributes so a response never emits both `result` and `error`, and
      `"jsonrpc": "2.0"` is always present.

**Hazard: MCP's stdio transport is newline-delimited JSON, not
`Content-Length`-framed like the Language Server Protocol.** One JSON object per
line, no headers, no blank-line separator. Implementing LSP framing here is the
single most common way to end up with an MCP server that never completes a
handshake and gives you nothing to debug.

---

## Step 2: `server.rs` — the stdio loop

- [ ] `pub fn serve<R: BufRead, W: Write>(reader: R, writer: W, store: &RuleStore) -> anyhow::Result<()>`.
      This crate is a transport adapter rather than part of the core engine, so
      `anyhow` is the right choice here and is already a declared dependency.
- [ ] Handle `initialize`, the `notifications/initialized` acknowledgement,
      `tools/list`, `tools/call` and `ping`.
- [ ] `initialize` result:
      `{ "protocolVersion": …, "capabilities": { "tools": {} }, "serverInfo": { "name": "opentaste", "version": env!("CARGO_PKG_VERSION") } }`.
      Echo the client's `protocolVersion` back when it is one you support, and
      fall back to `MCP_PROTOCOL_VERSION` otherwise.
- [ ] Notifications get **no response at all**. A response to a notification is a
      protocol violation that some clients treat as fatal.
- [ ] `ping` returns an empty result object.
- [ ] A line that fails to parse produces a `-32700` response and the loop
      **continues**. One bad line must never end the session.
- [ ] An unknown method produces `-32601`.
- [ ] Flush the writer after every response. A buffered reply that never leaves
      the process looks exactly like a hung server.
- [ ] Add `crates/taste-mcp/examples/stdio.rs` — three lines wiring
      `Paths::discover`, `RuleStore::open` and `serve(stdin().lock(),
      stdout().lock(), &store)`. It makes the server runnable, and therefore
      testable against a real client, before [Phase 5](PHASE-5.md) adds
      `taste mcp`.

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

- [ ] `tools/list` returns exactly these three, each with a `name`,
      `description` and a JSON Schema `inputSchema`.
- [ ] The third tool is named **`reject_coding_taste`**. SPEC.md's directory
      listing in section 2 calls it `reject_taste` while section 7 calls it
      `reject_coding_taste`; section 7 defines the actual interface, so it wins.
      Spell it one way everywhere, because the name is part of the wire contract
      and renaming it later silently breaks every editor already configured.
- [ ] Write `record_coding_taste`'s description as the instruction to the model it
      actually is: *"Call this tool immediately whenever the user corrects your
      code style, architecture, naming, or explicitly states a coding
      preference."* This string is the entire mechanism by which the loop gets
      triggered; a vague description means the tool is never called and the
      feature silently does not exist.
- [ ] `record_coding_taste` input: `category` (required, one of the seven
      slugs), `scope` (required glob), `instruction` (required), `anti_pattern`
      and `preferred_pattern` (optional).
- [ ] Validate `scope` with `storage::validate_scope` before recording. This is
      the one path where a glob arrives from outside the codebase.
- [ ] Parse `category` through `RuleCategory::from_str`, surfacing
      `Error::InvalidEnumValue` as a tool error listing the valid slugs.
- [ ] `reject_coding_taste` input: `rule_id` (required) and `delete`
      (optional, default `false`).
- [ ] After a successful `record_coding_taste`, call `RuleStore::sync()` and
      report what changed from the returned `SyncReport`, so the model can tell
      the user their preference reached their editor files.
- [ ] Return results as `{ "content": [{ "type": "text", "text": … }], "isError": false }`.

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

- [ ] A full handshake: `initialize`, then `notifications/initialized`,
      asserting the notification produced no output line.
- [ ] `tools/list` listing exactly three tools, with the names spelled as the
      spec spells them.
- [ ] `record_coding_taste` landing a rule in `.taste/rules.json` with
      `confidence = 0.95` and `source = "mcp-feedback"`, and the editor targets
      written.
- [ ] `get_coding_taste` returning only the rules whose scope matches the
      requested path.
- [ ] `reject_coding_taste` disabling by default, and deleting only with
      `"delete": true`.
- [ ] An unknown `rule_id` producing `isError: true` inside a *successful*
      response, not a JSON-RPC error.
- [ ] An invalid `scope` glob rejected with a message naming the offending
      pattern.
- [ ] A malformed line producing a `-32700` error and the loop surviving to
      answer the next valid request.
- [ ] `ping` answered.

---

## Definition of done

- [ ] `cargo fmt --all --check` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo test --workspace` clean.
- [ ] **Manual check:** a real handshake against Cursor with the server
      registered in `.cursor/mcp.json`, the three tools visible in the client,
      and a preference stated in chat landing in `.taste/rules.json`. Until
      [Phase 5](PHASE-5.md) ships `taste mcp`, register it with
      `cargo run -p taste-mcp --example stdio` or an equivalent temporary entry
      point.
- [ ] Status at the top of this file set to `completed`.
