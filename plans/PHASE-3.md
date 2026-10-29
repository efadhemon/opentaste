# Phase 3: Continuous integration, AST cold-start scanner and structural diff engine

**Status:** completed

| | |
| --- | --- |
| **Blocked by** | Nothing. `taste-core` phases 1 and 2 are complete. |
| **Blocks** | [Phase 5](PHASE-5.md) (`taste scan`, `taste learn-delta`), [Phase 6](PHASE-6.md) (needs the diff engine behind `learn-delta`) |
| **Independent of** | [Phase 4](PHASE-4.md) — the two share no code and may be built in either order |
| **Contains** | **Phase 0 (CI)**, part 0 below, which blocks *every* phase including 4 |
| **Rough size** | Part 0 is under an hour; the rest is 3 to 4 sessions |

This is the largest and least certain phase. Everything else in the project is
plumbing around it.

---

## Part 0 — Continuous integration (do this first)

148 tests protect nothing if nobody runs them, and every later phase adds more.
Do this before any phase, including phase 4.

The repository already has two commits on `main` and a remote at
`git@github.com:efadhemon/opentaste.git`, so the "make the first commit" step in
PLAN.md is already satisfied. CI lives in `.github/workflows/ci.yml`.

- [x] Create `.github/workflows/ci.yml` with one matrix job over
      `ubuntu-latest`, `macos-latest` and `windows-latest` on stable, running in
      this order: `cargo fmt --all --check`, `cargo clippy --workspace
      --all-targets -- -D warnings`, `cargo test --workspace`.
- [x] Add `Swatinem/rust-cache@v2` so the bundled SQLite in `rusqlite` is not
      recompiled from C on every run. Without it a cold Windows run is slow
      enough that people stop waiting for it.
- [x] Add a separate MSRV job pinned to `1.90` running `cargo check --workspace`.
      `tree-sitter` 0.27 declares `rust-version = "1.90"`, so the workspace floor
      moved with that dependency. `File::lock` still only needs 1.89; an
      undefended floor drifts upward silently the first time somebody uses a
      newer std API.
- [x] Trigger on `push` to `main` and on `pull_request`. Set
      `concurrency: { group: ${{ github.workflow }}-${{ github.ref }},
      cancel-in-progress: true }`.
- [x] Push `main` and confirm every job is green. Confirmed on `48277d3`.

Sketch:

```yaml
name: CI
"on":
  push:
    branches: [main]
  pull_request: {}
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true
jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace
  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@1.90
      - uses: Swatinem/rust-cache@v2
      - run: cargo check --workspace --all-targets
```

**Windows matters more than usual here.** Nothing in the workspace is
platform-locked — there is not a single `#[cfg(unix)]` or `#[cfg(windows)]`
branch — but every one of the 148 tests has only ever run on Linux. Four things
have never executed on Windows: the CRLF handling in `transpiler/marker.rs`, the
backslash normalisation in `normalize_path` in `storage/mod.rs`, the
`std::fs::File::lock` advisory lock in `storage/lock.rs`, and the rename in
`storage/atomic.rs`. The CRLF logic is the best defended of the four, since
`marker.rs` has tests that feed it literal `\r\n` input. Expect the first red
build to be Windows and treat it as a real bug, not a CI quirk.

**The most likely single failure is `fs::rename` in `atomic.rs`.** On Windows,
renaming over a file another process holds open fails with a sharing violation,
and antivirus scanners take transient handles on files as they are written. This
is the classic cause of intermittent "Access is denied" in Windows CI, and
`write_atomic` currently has no retry.

- [x] Windows CI on `48277d3` was green, so `write_atomic` has no retry.
      Add a short bounded retry — a handful of attempts with a few milliseconds
      between them — only if a later run shows the sharing violation, rather
      than abandoning the temp-file-and-rename approach, which is what gives
      the sync its crash safety. Keep any retry in `atomic.rs` so every target
      inherits it. The failure is intermittent, so one green run does not mean
      it is absent.

Two further notes on platform behaviour, neither of which needs action but both
of which are easy to "fix" wrongly later:

- The lock deliberately guards a dedicated `.taste/.lock` file rather than
  `rules.json` itself. `LockFileEx` on Windows is mandatory rather than
  advisory, so locking the rule file directly would block any process trying to
  read it. Do not simplify this away.
- `rusqlite` is declared with `bundled`, so every platform compiles SQLite from
  C and needs a C toolchain to build from source: MSVC on Windows, Xcode command
  line tools on macOS. End users installing through
  [Phase 6](PHASE-6.md)'s npm wrapper get prebuilt binaries and need neither.

**Gate for part 0:** all four jobs green on `main`.

---

## Part 1 — Dependencies

The compatibility risk flagged at the end of phase 2 is resolved, verified rather
than assumed: `tree-sitter` 0.27 works with the current 0.23-series grammar
crates. All four report ABI 14 and parse correctly. Entry points are
`tree_sitter_typescript::LANGUAGE_TYPESCRIPT`, `LANGUAGE_TSX`,
`tree_sitter_python::LANGUAGE`, `tree_sitter_rust::LANGUAGE` and
`tree_sitter_go::LANGUAGE`, each converted with `.into()`.

- [x] Add `tree-sitter-go = "0.23"` to `[workspace.dependencies]` in the root
      `Cargo.toml`. Every other crate needed here is already declared there.
- [x] Add to `crates/taste-core/Cargo.toml` `[dependencies]`: `tree-sitter`,
      `tree-sitter-typescript`, `tree-sitter-python`, `tree-sitter-rust`,
      `tree-sitter-go`, `ignore`, `rayon`, `similar` — all as
      `.workspace = true`.
- [x] Register the new modules in `crates/taste-core/src/lib.rs`: `pub mod ast;`
      and `pub mod diff;`, and re-export whatever the CLI will need.
- [x] Run `cargo tree -p taste-core -d` and confirm no duplicate `tree-sitter`
      versions were pulled in. Two ABI-incompatible copies in one binary fail at
      runtime with a confusing language-version error, not at compile time.

---

## Part 2 — The scanner (`ast/`)

### Step 1: `ast/parser.rs` — the multi-language wrapper

- [x] `pub enum SupportedLanguage { TypeScript, Tsx, JavaScript, Python, Rust, Go }`
      with `from_path(&Path) -> Option<Self>` driven by file extension, plus
      `name(&self) -> &'static str` for diagnostics.
- [x] `fn language(self) -> tree_sitter::Language` returning the grammar.
- [x] `pub fn parse(lang: SupportedLanguage, source: &str) -> Option<Tree>`.
- [x] `pub fn content_hash(bytes: &[u8]) -> String` using the `sha2` dependency
      that is already present. Step 3 needs it and so does `FileSnapshot`.

**Decision: `JavaScript` is a distinct variant that borrows the TSX grammar**
rather than a sixth grammar crate. `.ts` uses `LANGUAGE_TYPESCRIPT`, `.tsx` uses
`LANGUAGE_TSX`, and `.js`, `.jsx`, `.mjs` and `.cjs` use `LANGUAGE_TSX` too,
since it parses plain JavaScript and JSX and every analyzer looks at node shapes
the two languages share. Keeping the variant separate from `Tsx` still matters:
`types.rs` must **skip JavaScript files entirely**, because `interface` and
`type` cannot appear there and counting a file that could not have expressed
either would dilute the ratio. Note the grammar reuse in a comment so the next
reader knows it is a choice and not an oversight.

**Hazard: `tree_sitter::Parser` is neither `Sync` nor cheap to construct.** Do
not put one behind an `Arc` and do not build one per file. Give each worker
thread its own with `thread_local!`, built lazily per language. Call
`Parser::reset` only after a `parse` that returned `None` — by default the
parser resumes where a failed parse left off, so one unparseable file would
otherwise corrupt the next file that thread handles. It is not needed after a
successful parse.

### Step 2: `ast/analyzers/` — statistical detectors

Four modules, per SPEC.md section 2:

- [x] `imports.rs` — alias (`@/`, `~/`) versus deep relative (`../../`), named
      versus default exports.
- [x] `types.rs` — `interface X {}` versus `type X = {}`, skipping JavaScript
      files. Only aliases whose value is an object type count; a union or a
      function type cannot be an `interface` (review finding B5).
- [x] `functions.rs` — `function foo()` versus `const foo = () =>`, and the
      presence of early-return guard clauses.
- [x] `arch.rs` — custom decorators (`@AutoEntity`) versus framework ones
      (`@Entity`), and custom error classes versus bare `Error`. Import origin
      decides first (a package import is framework, a relative or alias import
      is the project's), then a name list. JavaScript, DOM and NestJS built-in
      exceptions are neither side (review finding B6).

**Scope narrowing, recorded on purpose.** SPEC.md section 2 also lists "strict
null checks" and "zod schemas" under `types.rs` and "DI patterns" under
`arch.rs`. PLAN.md already dropped all three. Strict null checking is a
`tsconfig.json` setting rather than a per-file statistical signal, and DI shape
varies too much between frameworks to count reliably. Leave all three out of
v0.1 rather than implementing weak versions, and see the deferred list in
[AGENTS.md](../AGENTS.md).

**Key decision: analyzers count, they do not decide.** Each returns a `Tally` of
raw counts for one file and never a rule. A separate `ast/promote.rs` folds
tallies together and converts them into `Observation`s only where the ratio is at
least `0.80` with support from at least 10 files, calling
`Observation::with_confidence(ratio)`.

- [x] `pub struct Tally` with named counters and an `impl AddAssign` (or a
      `merge`) so `rayon`'s `reduce` can fold per-file tallies without a lock.
      Track `files_contributing` per dimension, not just totals — the 10-file
      support floor is about files, not occurrences.
- [x] `ast/promote.rs` holding `MIN_RATIO: f32 = 0.80` and `MIN_FILES: usize = 10`
      as named constants, and one function per dimension turning a tally into
      `Option<Observation>` with the instruction text, scope glob, `RuleSource::AstScan`
      and the measured ratio as confidence.

Three reasons this split matters: per-file analysis stays pure and therefore
trivially parallel; the threshold policy lives in one reviewable place; and the
tallies themselves are what `taste status` should display, so a developer can see
*why* a rule was inferred.

**Recommendation: write the detectors as `tree_sitter::Query` patterns**, not
hand-rolled cursor walks. An S-expression query is a few lines, is readable by
someone who has never seen the file, and is far less likely to miss a node shape
than a manual recursion. Compile each query once per language into a
`thread_local!` or `OnceLock` alongside the parser. Reserve manual walking for
the control-flow analysis in part 3 step 5, where a query genuinely cannot
express the shape.

### Step 3: `ast/scan.rs` — the parallel repository walk

- [x] `pub fn scan(store: &RuleStore) -> Result<ScanReport>` as the single entry
      point, with `ScanReport { files_scanned, files_skipped, tallies, outcomes }`
      so `taste scan` and `taste status` can both report honestly.
- [x] Collect candidate paths with the serial `ignore::Walk` (respecting
      `.gitignore`), then analyse with `rayon::par_iter` over the collected
      paths.
- [x] Incremental rescan: for each path, hash the contents and skip the file when
      `SqliteStore::get_snapshot` reports the same `content_hash`;
      `upsert_snapshot` for the rest. This is exactly why the `file_snapshots`
      table was built in phase 1 and it has no other consumer.
- [x] Skip files above a size ceiling (1 MiB is ample) and anything that fails to
      decode as UTF-8, counting them as skipped rather than failing the scan.
- [x] Never walk vendored or built output (`node_modules`, `dist`, `build`,
      `out`, `vendor`, `coverage`, `target`, `.next`, …) or declaration and
      minified files (`*.d.ts`, `*.min.js`, `*.bundle.js`), with or without a
      `.gitignore` (review finding B7).
- [x] Detect the stack for `ProjectContext::detected_stack` by reading
      `package.json` (dependency names for `react`, `nestjs`, `next`, …),
      `Cargo.toml`, `pyproject.toml` and `go.mod`. Record it together with
      `total_files_scanned` through `RuleStore::set_project_context`.
- [x] Feed every promoted `Observation` through `RuleStore::record_observation`
      and collect the `Outcome`s. Do not write to `JsonStore` directly.

**Key decision: collect paths serially, then analyse in parallel.** `ignore`'s
`build_parallel()` uses its own thread pool with a callback API that does not
compose with rayon's `fold`/`reduce`, which forces a `Mutex` around the
accumulator. Walking serially and then driving `par_iter` over the collected
paths uses both crates honestly, needs no lock, and the walk is not the
bottleneck. Measured times are recorded in SPEC.md section 1.
This deliberately departs from SPEC.md section 8, which names
`ignore::WalkParallel`; the reason above is the reason, so do not "fix" it back.

**Hazard: `record_observation` takes the project lock on every call.** Aggregate
first and record the handful of promotable observations at the end. Calling it
per file would serialise the whole parallel scan on a file lock.

---

## Part 3 — The diff engine (`diff/`)

### Step 4: `diff/scrubber.rs` — build this before the detectors

Build the scrubber **first**, not after. The detectors in step 5 produce
`anti_pattern` and `preferred_pattern` snippets that get committed to
`.taste/rules.json`, and a leaked credential in a git-tracked file is the worst
failure this project can have. Writing the scrubber second leaves a window in
which the unsafe path exists.

- [x] `pub fn scrub(snippet: &str, lang: SupportedLanguage) -> String` replacing
      findings with `"<REDACTED_SECRET>"`, per SPEC.md section 5.3.
- [x] Shannon entropy above `4.5` on tokens longer than 16 characters, plus a
      3.0 limit for hex tokens of 32+ characters and a length-scaled limit for
      mixed-class tokens of 20+ characters. 4.5 alone cannot catch either
      (review finding B1).
- [x] Pattern matches for AWS access keys (`AKIA…`, `ASIA…`), other vendor
      prefixes (GitHub, Slack, Stripe, Google, GitLab, JWTs), webhook URLs,
      URL userinfo and secret query parameters, bearer tokens, private key
      headers (`-----BEGIN … PRIVATE KEY-----`), `.env`-style assignments
      quoted or bare (`SECRET=…`, `*_TOKEN=…`, `*_KEY=…`), and string literals
      assigned to names ending in a secret word (`password`, `apiKey`).
- [x] Call it from the one place every snippet passes through, so no future
      detector can bypass it. Assert that in a test.

**Hazard: entropy alone produces false positives** on legitimate code — long
hyphenated import paths, UUIDs in test fixtures, base64 asset data, hashes in
lockfile-like content. Two mitigations:

1. **Only consider the contents of string literals**, never identifiers or
   paths. tree-sitter already identifies string nodes precisely, so use it
   rather than a word-splitting regex over the whole snippet.
2. **Keep an allowlist of low-risk shapes**: bare UUIDs, semver strings, hex
   colours, and `sha256-`/`sha512-` integrity prefixes.

Redacting an identifier would destroy the snippet's usefulness as an example,
which is the entire reason it is stored.

### Step 5: `diff/structural.rs` — the seven detectors

- [x] A `Detector` trait taking `(before_tree, before_src, after_tree, after_src)`
      and returning `Option<Observation>`, plus
      `pub fn analyze(before: &str, after: &str, path: &Path) -> Result<Vec<Observation>>`
      that picks the language, parses both sides, and runs every detector.
- [x] Derive each `Observation`'s `scope` from the changed file's path, narrowed
      to a meaningful glob (`src/**/*.entity.ts` for `src/users/user.entity.ts`),
      not `**/*`. A rule scoped too widely is a wrong rule.
- [x] Cap every stored snippet at 12 lines and 400 bytes, scrubbing first and
      ending a cut snippet in a `// …` comment so it cannot contradict on a
      prefix. Skip any edit whose changed lines overlap an `ERROR` or
      `MISSING` node, since a half-typed buffer's recovered tree is a guess
      (review improvements 1 and 2).

**Recommendation: ship in two waves.** SPEC.md section 5.2 lists seven detectors
of very different difficulty. The first three are near-unambiguous and deliver
most of the value.

Wave one:

- [x] Decorator swap — `@A(...)` on a class, method or property becomes `@B(...)`,
      only when `A` is a framework decorator (known by name or imported from a
      package) and `B` is not. `@Get` to `@Post` is a behaviour change, not a
      taste (review finding B2).
- [x] Import path rewrite — deep relative becomes alias, or the reverse, using
      the scanner's own instruction constants so `opposite_instruction`
      recognises both. A sibling import (`./x`) is not either side (review
      finding B8).
- [x] Logger replacement — `console.log` / `console.error` becomes a logger
      service call (`this.logger.log`, `logger.info`).

Wave two:

- [x] Type construct swap — `interface X` ↔ `type X =`.
- [x] Function syntax preference — `function foo()` ↔ `const foo = () =>`.
- [x] Error class substitution — `throw new Error(...)` or `HttpException`
      becomes a custom exception class. A framework built-in or a class
      imported from a package does not count as custom (review finding B6).

Last, and only if the six above are solid:

- [x] Control-flow flattening — nested `if/else` refactored into early-return
      guard clauses. This needs real tree-shape comparison rather than node
      matching, and it is the one most likely to produce confident nonsense.
      Covers function declarations, class methods and `const` arrows, each
      keyed by its enclosing scopes (`UserService.load`) so same-named
      functions never pair (review findings B9, B11).

**Six working detectors beat seven where one misfires**, because a wrong rule
actively degrades every AI edit in the project. The control-flow detector
shipped: it fires only when the same function loses its `else` and gains an
early return. Editing the body of an `else` that stays an `else` does not fire.

### Step 6: `diff/contradiction.rs` — closing a gap in PLAN.md

SPEC.md section 5.1 specifies contradiction decay "if a developer commits code
matching the `anti_pattern` of an existing rule", and `apply_contradiction`
already exists for it, but no step in PLAN.md ever calls it. The detector belongs
here; the git-hook wiring that drives it belongs to
[Phase 5](PHASE-5.md) step 3.

- [x] `pub fn contradictions(store: &RuleStore, path: &Path, before: &str, after: &str) -> Result<Vec<String>>`
      returning the ids of active rules whose `anti_pattern` occurs more often
      in `after` than in `before`, in a file their scope matches. Comparing
      against `before` is what keeps untouched legacy code from decaying a
      rule on every commit that edits the same file (review finding B3).
- [x] Match structurally where the detector that created the rule allows it, and
      fall back to normalised-whitespace substring matching otherwise. Require a
      confident match: a false contradiction silently deletes a correct rule,
      which is harder to notice than a missing one.

### Step 7: The full test set

**Every step above lands with its own tests and a clean
`cargo test --workspace`; see [AGENTS.md](../AGENTS.md).** This is the complete
list the phase cannot ship without, so you can confirm nothing was skipped — not
a batch of work to leave until last. In this phase in particular, an untested
analyzer does not fail loudly: it produces a plausible-looking rule that is
wrong, and you will not notice four layers later.

- [x] Fixture-driven detector tests in
      `crates/taste-core/tests/fixtures/<detector>/{before,after}.ts`. Each
      detector gets a positive case **and** a negative case that must not fire.
- [x] A scrubber test where a snippet contains a credential, asserting the
      credential is gone and the surrounding identifiers survive intact.
- [x] A scrubber test over the allowlist shapes asserting a UUID and a semver
      string are *not* redacted.
- [x] A scanner test over a small synthetic repository asserting that a 9-of-10
      ratio promotes and a 6-of-10 ratio does not.
- [x] A rescan test asserting the second `scan` of an unchanged repository
      reports every file skipped and records no new observations.
- [x] A parser test asserting every `SupportedLanguage` parses a minimal sample
      without error, which is what will catch a grammar ABI mismatch early.

---

## Definition of done

- [x] Part 0's four CI jobs green on `main` (`48277d3`).
- [x] `cargo fmt --all --check` clean.
- [x] `cargo clippy --workspace --all-targets -- -D warnings` clean.
- [x] `cargo test --workspace` clean, with the 148 existing tests still passing.
- [x] **Manual check, not skippable:** run the scanner against a real TypeScript
      project and read the rules it produces. A human has to agree with them.
      This is the only gate that catches a detector that is confidently wrong,
      and it is not automatable. Ran on `crisis-desk-ai` (NestJS, 134 files).
      One rule promoted: "Prefer named exports over default exports." at
      confidence 0.98 (223 named, 4 default, 124 files). The other six
      dimensions stayed under the 10-file / 0.80 floor. Agreed.
- [x] Record the measured cold scan time on that project against SPEC.md section
      1's sub-300ms claim. If it misses, write down the real number here rather
      than quietly leaving the spec wrong. `crisis-desk-ai`: 157 ms, debug
      build, 134 files parsed, 0 skipped. A second tree, `cortex` (991 files),
      took 1260 ms in debug and 321 ms in release. The release figure is the
      one to compare with the spec, and it is over 300 ms, so SPEC.md section 1
      now states both measurements instead of an unqualified `<300ms`.
- [x] Status at the top of this file set to `completed`, and any detector left
      unshipped recorded above with the reason. All seven detectors shipped.
