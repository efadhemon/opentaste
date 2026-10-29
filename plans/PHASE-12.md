# Phase 12: Dashboard editing, highlighting and scan speed

**Status:** pending

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

- [ ] `e` on a rule opens an edit mode for its **instruction** and **scope**,
      one field at a time, with the usual line-editing keys; `Enter` saves,
      `Esc` cancels.
- [ ] A scope is checked with `storage::validate_scope` before saving; an
      invalid glob is shown as an error and nothing is written.
- [ ] Saving goes through a new `RuleStore::update_rule(id, instruction,
      scope)`, under the lock, then syncs. Editing a candidate updates the
      cache; editing a confirmed rule updates `rules.json`.
- [ ] If the new wording makes the rule a duplicate of another, say so and
      do not save; merging two rules is not an edit.
- [ ] The status bar lists `[e] Edit`.
- [ ] Tests, against the terminal-free `App` state machine: edit and save
      each field; cancel leaves the rule untouched; an invalid scope is
      refused; a duplicate is refused; the edited rule reaches the editor
      files.

## Step 2: Syntax highlighting (M8)

SPEC.md §8: a "syntax-highlighted `Anti-Pattern (Avoid)` vs
`Preferred-Pattern (Use)` diff box". Today the two snippets are red and green.

- [ ] Highlight with `tree-sitter-highlight` (0.27, matching the workspace's
      `tree-sitter`) and each grammar's bundled `HIGHLIGHTS_QUERY`, so no new
      grammar or highlighter library is added.
- [ ] The language comes from the rule's scope extension; an unknown one
      falls back to the current colouring.
- [ ] Keep the avoid/prefer distinction with the box border and title, now
      that the text itself is coloured by syntax.
- [ ] Respect `NO_COLOR` and the existing ASCII fallback.
- [ ] Tests: a TypeScript snippet gets keyword and string styles; an unknown
      extension falls back; `NO_COLOR` gets no styles.

## Step 3: Parallel walk and the 300 ms target (M9)

SPEC.md §1 and §8: `ignore::WalkParallel`, "<300ms". Today the walk and the
read-and-hash loop are serial; 991 files took 321 ms in a release build.

- [ ] Walk with `WalkParallel`, read and hash in the walker threads, and
      load every snapshot into a `HashMap` once instead of one query per
      file.
- [ ] Keep the result deterministic: sort the collected files before
      tallying and before writing snapshots, so two scans of the same tree
      write identical caches and rules.
- [ ] A benchmark that generates a 1,000-file tree and times a cold and a
      warm scan in release mode, runnable as one command and recorded here.
- [ ] **Target:** cold scan of 1,000 files under 300 ms on a 4-core machine
      in release mode. If a language from phase 8 makes it impossible, SPEC.md
      §1 states the measured number per language instead of the claim.
- [ ] Tests: two scans of the same tree produce byte-identical
      `rules.json`; results equal the serial implementation's on the
      existing fixtures.

## Step 4: The full test set

- [ ] Edit: every field, cancel, invalid scope, duplicate, sync.
- [ ] Highlighting: styles, fallback, `NO_COLOR`.
- [ ] Scan: determinism and equality with the serial results.

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
