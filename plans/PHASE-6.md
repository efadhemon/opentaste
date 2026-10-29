# Phase 6: Editor extension and npm wrapper

**Status:** pending

| | |
| --- | --- |
| **Blocked by** | [Phase 5](PHASE-5.md) — needs a working `taste` binary and the frozen `learn-delta` contract; [Phase 3](PHASE-3.md) transitively, since `learn-delta` has nothing to say without the diff engine; **Phase 0 (CI)**, part 0 of [PHASE-3.md](PHASE-3.md) |
| **Blocks** | Nothing. This is the last phase. |
| **Rough size** | 2 sessions |

The first TypeScript in the project, and the first time OpenTaste runs inside an
editor rather than beside it.

---

## Step 1: TypeScript workspace scaffolding

SPEC.md section 2 lists a `pnpm-workspace.yaml` and a root `package.json` that do
not exist yet.

- [ ] `pnpm-workspace.yaml` with `packages: ["packages/*"]`.
- [ ] Root `package.json` holding only dev tooling and scripts — no runtime
      dependencies at the root.
- [ ] `packages/vscode-opentaste/` with `package.json`, `tsconfig.json` and
      `src/`.
- [ ] `packages/npm-opentaste/` with `package.json` and `bin/`.
- [ ] `.gitignore` already covers `node_modules/`, `dist/`, `out/`, `*.vsix`,
      `*.tsbuildinfo` and the pnpm store. Nothing to add.
- [ ] Set `engines.vscode` low enough that the Cursor and Windsurf forks accept
      it. `^1.85.0` is a safe floor; a recent `^1.9x` requirement silently
      excludes the forks this extension exists to support.
- [ ] Extend the CI workflow from [Phase 3](PHASE-3.md) part 0 with a Node job
      running `pnpm install --frozen-lockfile`, `tsc --noEmit` and the extension
      tests. A TypeScript package outside CI will break and nobody will notice.
- [ ] Commit `pnpm-lock.yaml` in the same change as that job, or
      `--frozen-lockfile` fails on its first run.

---

## Step 2: `deltaTracker.ts` — the hard part

The detection heuristic from SPEC.md section 9: an insertion of at least four
lines in one transaction opens an `AiBurstWindow`; small subsequent edits inside
that range within 180 seconds, followed by a save, trigger `taste learn-delta`.

- [ ] `AiBurstWindow { filePath, startLine, endLine, aiSnapshotText, timestamp }`,
      opened from `vscode.workspace.onDidChangeTextDocument` when one transaction
      inserts ≥ 4 lines.
- [ ] Mark the window as corrected on subsequent small edits (roughly 1–15
      characters, deletions, or line modifications) inside `startLine..endLine`.
- [ ] On `onDidSaveTextDocument`, if a corrected window is still within its 180
      second lifetime, extract `before` (`aiSnapshotText`) and `after` (the
      current text of that range) and hand both to `binaryRunner`.
- [ ] Expire windows on a timer as well as on save, so a file left open for an
      hour does not carry a stale window.
- [ ] Cap the number of live windows per document. An agent making many bursts in
      one file must not grow this without bound.

**Three hazards, each of which produces wrong rules if missed:**

- **Undo and redo look exactly like AI bursts.** A single undo can reinsert
  dozens of lines in one transaction. Check `event.reason` against
  `vscode.TextDocumentChangeReason.Undo` and `.Redo` and skip those events
  entirely. Without this, every undo teaches OpenTaste the reverse of what the
  developer wanted.
- **OpenTaste's own writes fire change events.** `CLAUDE.md`, `.windsurfrules`,
  `AGENTS.md`, `.github/copilot-instructions.md` and the `.cursor/rules/*.mdc`
  files all change during `sync`. Filter to source files and exclude the
  transpiler's targets explicitly — the same list `config::Paths` owns, not a
  second hardcoded copy that will drift. Phase 5 put that list in core as
  `Paths::marker_targets` and `Paths::is_generated_target` (marker files plus
  `.cursor/rules/opentaste-*.mdc`), but the binary does not print it yet. Add
  a small `taste targets --json` reading those two, rather than copying the
  paths into TypeScript.
- **Window line ranges drift.** An edit *above* an open burst window shifts the
  tracked range downward or upward. Adjust `startLine` and `endLine` on every
  intervening change, or the captured "after" text will be a completely
  different block of code and the resulting rule will be nonsense.

---

## Step 3: `binaryRunner.ts` and `statusBar.ts`

- [ ] Spawn the local `taste` binary over `child_process`, writing
      `{ "before": …, "after": … }` to stdin and parsing the response from
      stdout. **This is the contract fixed in [Phase 5](PHASE-5.md) step 3** —
      `{ promoted: TasteRule[], candidates: {…}[] }`, both always present.
      Each `promoted` entry is a `TasteRule` exactly as in `rules.json`, so
      `anti_pattern` and `preferred_pattern` are absent when the rule has
      none; type them as optional. Each `candidates` entry is exactly
      `{ id, confidence, instruction }`.
- [ ] Tolerate an empty stdout with a non-zero exit code: that is how the binary
      reports failure, and it must be silent rather than a popup.
- [ ] Pass `--file` as a path relative to the workspace folder, and spawn the
      child with that folder as its working directory. `RuleStore::rules_for_file`
      relativises against the project root, and `Paths::discover` walks up from
      the working directory, so an absolute path from a different root silently
      matches nothing.
- [ ] Use `onDidSaveTextDocument`, not `onWillSaveTextDocument`. SPEC.md section
      9 mentions both, but only the former sees the text that was actually saved.
- [ ] Resolve the binary in this order: the workspace, then `PATH`, then the
      bundled download. Fail quietly when it is absent — **never nag**. A tool
      whose promise is staying out of the way cannot open a modal on startup.
- [ ] Time out and kill the child process. A hung binary must not leak a process
      per save.
- [ ] `statusBar.ts` — a status bar item showing the active rule count, and the
      non-intrusive toast from SPEC.md section 9:
      `✨ OpenTaste learned: "…" [Keep Rule] [Ignore]`, shown only when
      `promoted` is non-empty.
- [ ] `[Ignore]` maps onto `reject_coding_taste` / `set_enabled(id, false)`, and
      `[Keep Rule]` simply dismisses. Nothing new is needed in the core for
      either.
- [ ] `extension.ts` — lifecycle and command registration, with every disposable
      pushed onto `context.subscriptions`.

---

## Step 4: `npm-opentaste` and release automation

`bin/taste.js` resolves `process.platform` and `process.arch` and executes the
matching prebuilt binary. That implies a release pipeline, which the spec does
not mention but which `npx opentaste` cannot work without.

**Key decision: ship per-platform packages as `optionalDependencies`, not a
postinstall download.** One package per target (`@opentaste/cli-linux-x64`, and
so on) with `os` and `cpu` fields, listed as optional dependencies of
`opentaste`; npm installs only the matching one. A postinstall script that
fetches from GitHub releases breaks under `--ignore-scripts`, breaks behind
corporate proxies, and is exactly the kind of network call a tool advertising
"100% offline execution, zero telemetry" should not make.

- [ ] `bin/taste.js` resolving the platform package and `execFileSync`-ing its
      binary, passing through argv, stdio and the exit code. It must be
      transparent enough that `npx opentaste mcp` works as an MCP server, which
      means **not writing anything to stdout itself**.
- [ ] A release workflow building the matrix: `x86_64-unknown-linux-gnu`,
      `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`,
      `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- [ ] **Build each target on a native runner, not by cross-compiling.**
      `rusqlite` is declared with the `bundled` feature, so every build compiles
      SQLite from C. Cross-compiling therefore needs a full cross C toolchain,
      not just `rustup target add`, and that is a deep hole to fall into on
      release day. Use `ubuntu-24.04-arm` and `macos-14` for the arm64 targets.
- [ ] Attach the binaries to a tagged GitHub release, and publish the platform
      packages plus the `opentaste` wrapper to npm.
- [ ] Publish the extension to **both** the VS Code Marketplace (`vsce`) and Open
      VSX (`ovsx`). SPEC.md section 9 targets Windsurf and Cursor users, who can
      only install from the latter.
- [ ] Keep the release workflow separate from CI and triggered on tags only.

---

## Step 5: The full test set

**Every step above lands with its own tests and a clean run of the Node job;
see [AGENTS.md](../AGENTS.md).** This is the complete list the phase cannot ship
without, so you can confirm nothing was skipped — not a batch of work to leave
until last. `deltaTracker` is the one to be strictest about: each of its three
hazards produces a confidently wrong rule rather than an error, and none of them
is visible without a test that reproduces the event sequence.

- [ ] Unit-test `deltaTracker` against synthesised change events: a genuine
      4-line burst opens a window; an undo-flagged event does not; an edit above
      an open window shifts its range correctly; a write to `CLAUDE.md` is
      ignored; a window older than 180 seconds does not fire.
- [ ] Unit-test `binaryRunner`'s parsing against the exact JSON from
      [Phase 5](PHASE-5.md) step 3, including the empty `{"promoted":[],"candidates":[]}`
      case and the non-zero-exit case.
- [ ] Test `bin/taste.js`'s platform resolution for each supported
      platform/arch pair and the unsupported-platform message.

---

## Definition of done

- [ ] CI's Node job green: `tsc --noEmit` clean and extension tests passing.
- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` still clean.
- [ ] **Manual check:** install the packaged extension in VS Code, Cursor **and**
      Windsurf, and confirm in each that a real AI edit followed by a human
      correction produces a learned rule and the toast.
- [ ] `npx opentaste status` works from a clean machine with no Rust toolchain.
- [ ] Status at the top of this file set to `completed`.
