# Phase 5: CLI and terminal dashboard

**Status:** pending

| | |
| --- | --- |
| **Blocked by** | **Phase 0 (CI)** — part 0 of [PHASE-3.md](PHASE-3.md); [Phase 3](PHASE-3.md) for `scan` and `learn-delta`; [Phase 4](PHASE-4.md) for `mcp` |
| **Blocks** | [Phase 6](PHASE-6.md) — needs a working `taste` binary and the `learn-delta` contract fixed in step 3 |
| **Rough size** | 2 sessions |

This phase produces the artefact everything else is judged by: the `taste`
binary. `crates/taste-cli/src/main.rs` currently prints the version and nothing
else.

---

## Part 0 — Dependencies

### Verified version notes

`ratatui` 0.30 has been split into `ratatui-core` plus backend crates and exposes
explicit `crossterm_0_28` / `crossterm_0_29` features. Declare
`ratatui = { version = "0.30", features = ["crossterm_0_29"] }` and **import
crossterm through the `ratatui::crossterm` re-export** rather than depending on
`crossterm` directly. Depending on both gets you two incompatible crossterm
versions in the tree and type errors that do not mention the real cause.

`notify`'s latest stable is 8.2; 9.0 is still a release candidate, so stay on 8.

- [ ] Add to `crates/taste-cli/Cargo.toml` `[dependencies]`: `clap`, `ratatui`,
      `notify`, `serde`, `serde_json`, `chrono` as `.workspace = true`, beside
      the existing `taste-core`, `taste-mcp` and `anyhow`.
- [ ] Do **not** add `crossterm` to this crate.
- [ ] Add `[dev-dependencies]`: `tempfile`, `pretty_assertions`.
- [ ] For the integration tests, drive the binary through
      `std::process::Command::new(env!("CARGO_BIN_EXE_taste"))` rather than
      adding `assert_cmd`. It needs no new dependency and cargo guarantees the
      path.

---

## Step 1: `main.rs` and the command skeleton

- [ ] Wire `clap` derive for `init`, `scan`, `sync`, `learn-delta`, `learn`,
      `status`, `mcp`, `ui` and `watch`.
- [ ] Resolve the project with `Paths::discover` and open a `RuleStore` **once,
      in one place**, then hand `&RuleStore` to each command. `init` is the
      exception: it runs before a project exists.
- [ ] Use `anyhow` here, as planned. `taste-core` returns typed errors and the
      binary is where they become messages a human can act on — in particular
      `Error::ProjectRootNotFound` should say "run `taste init` first", which it
      already does.
- [ ] Add a global `--quiet` flag, because the git hook and the editor extension
      both invoke the binary non-interactively.

---

## Step 2: `commands/init.rs` — the most dangerous command

Five actions per SPEC.md section 8, and three of them touch files the developer
owns. Apply the same discipline `transpiler/marker.rs` already follows: never
clobber, and refuse rather than guess.

- [ ] Create `.taste/`, `rules.json` and `cache.db`.
- [ ] Append the cache entries to `.gitignore` **idempotently**. Running `init`
      twice must not duplicate the lines. Match on the entry text, not on a
      whole-block comparison, so a developer who reordered their `.gitignore`
      does not get a second copy.
- [ ] Register the server in `.cursor/mcp.json` by **merging into the existing
      JSON**: insert `mcpServers.opentaste = { "command": "taste", "args": ["mcp"] }`
      and preserve every other key and every other server. Blindly writing this
      file deletes a developer's other MCP servers, which is the phase 5
      equivalent of eating `CLAUDE.md`. If the file exists but is not valid
      JSON, stop and print what to add by hand — do not "repair" it.
- [ ] Install `.git/hooks/post-commit` **without overwriting an existing hook**.
      Append a marked section exactly as `marker.rs` does for `CLAUDE.md`, or
      stop and tell the user the line to add. Set the executable bit on Unix;
      skip that silently on Windows.
- [ ] Handle a project root with no `.git` directory. `Paths::discover` falls
      back to a `.taste` directory, so `init` can legitimately run outside a git
      repository: skip the hook with a one-line note and carry on rather than
      failing the whole command.
- [ ] Run `scan` then `sync`, and report what both did.
- [ ] Make the whole command re-runnable. `taste init` on an already-initialised
      project must be a no-op that says so, not an error and not a reset.

---

## Step 3: The remaining commands

`scan` and `sync` are thin wrappers over [Phase 3](PHASE-3.md)'s `ast::scan` and
`RuleStore::sync`.

- [ ] `scan` — print the `ScanReport`: files scanned, files skipped, and every
      promoted rule.
- [ ] `sync` — print the `SyncReport`. It already distinguishes `written`,
      `unchanged` and `removed`, so say which editor targets changed rather than
      just claiming success.
- [ ] `status` — the summary table from SPEC.md section 8: confirmed rules with
      confidence and occurrences, candidate rules, the detected stack from
      `ProjectContext`, and which editor targets are currently up to date.
      Phase 3's tallies belong here too, so a developer can see *why* a rule was
      inferred.
- [ ] `mcp` — `taste_mcp::serve(stdin().lock(), stdout().lock(), &store)`.
      Nothing else, and no logging to stdout.
- [ ] `watch` — `notify` with a debounce. Note that `notify` 8 ships **no
      debouncer**; it lives in the separate `notify-debouncer-full` crate. Hand-
      roll a simple time-window debounce rather than adding another dependency
      for one timer. The transpiler already skips writes whose content is
      unchanged, which is most of what keeps the watcher from reacting to its own
      output, but **still filter OpenTaste's own target paths explicitly** rather
      than relying on that alone. Read the target list from `config::Paths` so it
      cannot drift. Tune the debounce window on macOS as well as Linux: `notify`
      uses FSEvents there and inotify here, and the two coalesce events
      differently, so a window that feels right on one can double-fire on the
      other.
- [ ] `learn --from-git-head` — the command the `post-commit` hook installed in
      step 2 actually invokes. SPEC.md section 8 specifies the hook as
      `taste learn --from-git-head &`, but PLAN.md's command list omits `learn`
      entirely; this is the one place the two documents disagree, and this is the
      resolution. It diffs `HEAD` against `HEAD~1`, feeds each changed source
      file's before/after through `diff::structural::analyze`, and runs
      [Phase 3](PHASE-3.md) step 6's `diff::contradiction::contradictions`
      against the new content so `RuleStore::apply_contradiction` finally has a
      caller. Broader "recent git history" mining, mentioned in SPEC.md section
      1, is **out of scope for v0.1** — record that here rather than
      half-building it.

### `learn-delta` — the cross-phase contract

**This output shape is consumed by [Phase 6](PHASE-6.md) step 3. Settle it here
and change it in both files or not at all.**

- [ ] Invocation: `taste learn-delta --file <relative_path>`, payload on stdin.
- [ ] Stdin, exactly as SPEC.md section 8 specifies:

```json
{ "before": "…", "after": "…" }
```

- [ ] Stdout: **one** JSON object on a single line, always valid JSON, always
      exit code 0 when the analysis ran — including when it learned nothing:

```json
{
  "promoted": [
    {
      "id": "arch-custom-entity-decorators-a1b2c3",
      "category": "architecture",
      "scope": "src/**/*.entity.ts",
      "confidence": 0.95,
      "occurrences": 3,
      "enabled": true,
      "source": "ast-diff",
      "instruction": "Use custom @AutoEntity() decorators instead of raw @Entity().",
      "anti_pattern": "@Entity('users')",
      "preferred_pattern": "@AutoEntity()",
      "updated_at": "2026-10-29T00:00:00Z"
    }
  ],
  "candidates": [
    { "id": "types-interface-over-alias-9f8e7d", "confidence": 0.56, "instruction": "…" }
  ]
}
```

- [ ] `promoted` entries are serialised `TasteRule`s verbatim, so the shape stays
      in step with `.taste/rules.json` for free. `promoted` and `candidates` are
      always present and may be empty arrays.
- [ ] Nothing else ever goes to stdout. Errors go to stderr with a non-zero exit
      code and an empty stdout, which phase 6 must tolerate.
- [ ] Snippets in the output have already passed through
      [Phase 3](PHASE-3.md)'s scrubber, because they came back out of the store.

---

## Step 4: `tui/app.rs` and `tui/ui.rs`

**Key decision: state lives in `app.rs`, rendering lives in `ui.rs`, and the
state machine has no terminal dependency.** Navigation, filtering, toggling,
promotion and deletion are then unit-testable without a PTY, which is the
difference between a tested dashboard and a hand-tested one.

- [ ] `App` holding the loaded rules and candidates, the selection index, the
      filter string and a status message, with methods for each action and no
      `ratatui` or `crossterm` types in its signatures.
- [ ] Layout per SPEC.md section 8: left pane 45% (filterable list of confirmed
      `[✓]` and candidate `[?]` rules with a confidence bar and category badge),
      right pane 55% (scope, occurrences, instruction, and the anti-pattern
      versus preferred-pattern box), bottom status bar listing the keys.
- [ ] Keybindings: `↑`/`↓` navigate, `[space]` toggle enabled, `[p]` promote
      candidate, `[d]` delete, `[s]` sync, `[q]` quit. Map them onto
      `set_enabled`, `promote_candidate`, `delete_rule` and `sync`.
- [ ] Require a confirmation keystroke for `[d]`. Deletion is the one
      irreversible action in the dashboard.
- [ ] Check the `[✓]`, `[?]` and `█` glyphs on Windows. The legacy conhost with
      a non-UTF-8 codepage renders them as mojibake; Windows Terminal is fine.
      Either accept that and say so, or keep an ASCII fallback — but decide,
      rather than finding out from a bug report.

**Hazard: install a panic hook that restores the terminal** — leaving raw mode
and the alternate screen — **before any `ratatui` code runs**, and chain to the
previous hook so the backtrace still prints. Without it a panic leaves the user
with an unusable shell and no error message, which is a terrible first impression
for a tool whose whole promise is staying out of the way. Note that the release
profile sets `panic = "abort"`; the hook still runs, so this remains worth doing.

---

## Step 5: The full test set

**Every step above lands with its own tests and a clean
`cargo test --workspace`; see [AGENTS.md](../AGENTS.md).** This is the complete
list the phase cannot ship without, so you can confirm nothing was skipped — not
a batch of work to leave until last. `init` is the command to be strictest
about: each of its five actions touches something the developer owns, and a
regression there destroys their files rather than merely misbehaving.

- [ ] Unit-test the `App` state machine directly: navigation at the list
      boundaries, filtering, toggle, promote, delete-with-confirmation.
- [ ] Integration test for `init` asserting an existing `.cursor/mcp.json` keeps
      its other servers and an existing `post-commit` hook keeps its contents.
- [ ] Integration test for `init` run twice, asserting `.gitignore` gains each
      entry exactly once.
- [ ] Integration test for `init` against a `.cursor/mcp.json` containing invalid
      JSON, asserting the file is left untouched and the command explains what
      to do.
- [ ] Integration test for `learn-delta` feeding a known before/after pair on
      stdin and asserting the exact JSON contract above, including the
      learned-nothing case.
- [ ] Integration test asserting `taste` outside any project exits non-zero with
      a message naming `taste init`.

---

## Definition of done

- [ ] `cargo fmt --all --check` clean.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [ ] `cargo test --workspace` clean.
- [ ] **Manual check:** a `taste ui` session — navigate, toggle a rule, sync, and
      quit, confirming the terminal is left in a usable state. Then panic it on
      purpose once and confirm the same.
- [ ] The `learn-delta` contract in step 3 is final, and [Phase 6](PHASE-6.md)
      step 3 matches it.
- [ ] Status at the top of this file set to `completed`.
