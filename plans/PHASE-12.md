# Phase 12: Dashboard editing, highlighting and scan speed

**Status:** ongoing

| | |
| --- | --- |
| **Blocked by** | [Phase 8](PHASE-8.md) for step 3 (speed is measured with every language on). Steps 1 and 2 only need phase 7. |
| **Blocks** | Nothing |
| **Rough size** | 2 sessions |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) M7, M8, M9 |

---

## Step 1: Edit a rule from the dashboard (M7)

SPEC.md §2 describes `tui/app.rs` as "Rule list, filter, toggle, **edit**,
delete". Everything but edit exists.

- [x] `e` on a rule opens an edit mode for its **instruction** and **scope**,
      one field at a time, with the usual line-editing keys; `Enter` saves,
      `Esc` cancels. (`Tab` switches field; ←/→, Home/End, Backspace and
      Delete edit around a cursor. A refused save keeps the edit open.)
- [x] A scope is checked with `storage::validate_scope` before saving; an
      invalid glob is shown as an error and nothing is written.
- [x] Saving goes through a new `RuleStore::update_rule(id, instruction,
      scope)`, under the lock, then syncs. Editing a candidate updates the
      cache; editing a confirmed rule updates `rules.json`. (The identifier
      stays the same: an edit corrects a rule rather than making a new one,
      so `taste disable`, contradictions and later sightings still find it.)
- [x] If the new wording makes the rule a duplicate of another, say so and
      do not save; merging two rules is not an edit. (Duplicate by
      deduplication's rules, or the same words over the same scope in any
      category, which would print the same line twice. An empty instruction
      is refused too.)
- [x] The status bar lists `[e] Edit`.
- [x] Tests, against the terminal-free `App` state machine: edit and save
      each field; cancel leaves the rule untouched; an invalid scope is
      refused; a duplicate is refused; the edited rule reaches the editor
      files. (Plus `update_rule`'s own tests in `promotion_flow.rs`.)

## Step 2: Syntax highlighting (M8)

SPEC.md §8: a "syntax-highlighted `Anti-Pattern (Avoid)` vs
`Preferred-Pattern (Use)` diff box". Today the two snippets are red and green.

- [x] Highlight with `tree-sitter-highlight` (0.27, matching the workspace's
      `tree-sitter`) and each grammar's bundled `HIGHLIGHTS_QUERY`, so no new
      grammar or highlighter library is added. (`tui/highlight.rs`.
      TypeScript's bundled query only covers what TypeScript adds to
      JavaScript, and `tree-sitter-javascript` is not a dependency, so
      `SupportedLanguage::highlights_query` prepends a short query for the
      shared captures: strings, comments, numbers, keywords, calls. A test
      compiles every language's query against its grammar.)
- [x] The language comes from the rule's scope extension; an unknown one
      falls back to the current colouring. (`SupportedLanguage::from_scope`,
      shared with deduplication.)
- [x] Keep the avoid/prefer distinction with the box border and title, now
      that the text itself is coloured by syntax.
- [x] Respect `NO_COLOR` and the existing ASCII fallback. (With `NO_COLOR`
      the snippets and their borders are plain; the ASCII glyphs are
      unchanged.)
- [x] Tests: a TypeScript snippet gets keyword and string styles; an unknown
      extension falls back; `NO_COLOR` gets no styles. (Also Python, Rust
      and Go keywords, and the text surviving highlighting unchanged.)

## Step 3: Parallel walk and the 300 ms target (M9)

SPEC.md §1 and §8: `ignore::WalkParallel`, "<300ms". Today the walk and the
read-and-hash loop are serial; 991 files took 321 ms in a release build.

- [x] Walk with `WalkParallel`, read and hash in the walker threads, and
      load every snapshot into a `HashMap` once instead of one query per
      file. (Snapshots are also written, and stale ones pruned, in one
      SQLite transaction each instead of one commit per file.)
- [x] Keep the result deterministic: sort the collected files before
      tallying and before writing snapshots, so two scans of the same tree
      write identical caches and rules.
- [x] A benchmark that generates a 1,000-file tree and times a cold and a
      warm scan in release mode, runnable as one command and recorded here.
      (`taste-core/tests/scan_bench.rs`: 700 TypeScript, 100 each of
      Python, Rust and Go, every TypeScript file importing zod and NestJS, so
      every analyzer runs.)

      ```bash
      taskset -c 0-3 cargo test --release -p taste-core --test scan_bench -- --ignored --nocapture
      ```
- [x] **Target:** cold scan of 1,000 files under 300 ms on a 4-core machine
      in release mode. If a language from phase 8 makes it impossible, SPEC.md
      §1 states the measured number per language instead of the claim.

      **The walk was not the bottleneck.** Timing each phase showed the walk
      and read at 34 ms, snapshot writes at 34 ms, and analysis at 560 ms on
      four cores: about 2.4 ms per TypeScript file on top of a 0.3 ms parse.
      Running a tree-sitter query costs about as much as walking every node,
      whatever its pattern count (one pattern 129 µs, eleven 148 µs), and a
      TypeScript file ran ten. So, besides the parallel walk:

      - the six whole-tree script queries run as one combined query
        (`analyzers::Pass`); each analyzer keeps its own query text and reads
        its matches back in its own capture numbering, and a test checks the
        combined pass finds exactly what each query finds alone;
      - import queries stop at the top of the module (`ScriptQuery::top_level`),
        and a file's import origins are computed once instead of twice;
      - the validation and DI analyzers return early for files that import
        no validation library or DI framework, and DI looks only at
        top-level classes (`TALLY_FORMAT` → `tally5`).

      | 1,000 files, release, 4 cores (`taskset -c 0-3`) | Cold | Warm |
      | --- | --- | --- |
      | Before this step | 750–900 ms | 37–59 ms |
      | After | **282–294 ms** | **12–14 ms** |

      Measured 2026-10-31 on a 12-core x86-64 Linux machine restricted to
      four cores. All twelve cores: 451 ms before.

      **The target is met with little margin, and the number moves with
      machine load.** Re-measured after merging phase 11 into this branch:
      263–310 ms cold over four runs. At the same moment the pre-merge commit
      (`98c90f5`) measured 314–332 ms, so the merge did not regress it; the
      machine was busier. Treat "under 300 ms" as the typical result on four
      idle cores, not a guarantee.
- [x] Tests: two scans of the same tree produce byte-identical
      `rules.json`; results equal the serial implementation's on the
      existing fixtures. (Rules and snapshots compared without their
      timestamps across six scans of a tree spread over many directories;
      the combined query pass compared with each query run alone.)

## Step 4: The full test set

- [x] Edit: every field, cancel, invalid scope, duplicate, sync.
- [x] Highlighting: styles, fallback, `NO_COLOR`.
- [x] Scan: determinism and equality with the serial results.

---

## Definition of done

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean.
- [ ] **Manual gate:** edit a rule in `taste ui` in a real terminal and see
      the change in `CLAUDE.md`; check highlighting in a dark and a light
      terminal theme.
- [ ] The benchmark numbers recorded here and in SPEC.md §1.
- [ ] V1-AUDIT M7, M8 and M9 marked fixed.
- [ ] Status at the top of this file set to `completed`.
