# Phase 8: Python, Rust and Go

**Status:** ongoing

| | |
| --- | --- |
| **Blocked by** | [Phase 7](PHASE-7.md): language gating (`has_analyzers`), versioned tallies, package scopes |
| **Blocks** | [Phase 10](PHASE-10.md) (history mining uses every detector), [Phase 12](PHASE-12.md) (speed is measured with all languages on) |
| **Rough size** | 4–5 sessions, one per language plus shared groundwork |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) M2, M12 |

SPEC.md §2 and §10 promise analyzers for TS/JS, Python, Rust and Go. All four
grammars already parse; Python, Rust and Go produce no counts and no
observations. This phase gives each of them the conventions that are real,
countable choices in that language, held to the same rule as TypeScript:
**when a detector is ambiguous, it does not emit.**

## Which conventions, per language

A dimension is only listed if both styles are legal, common, and a
reasonable team could choose either. Where a language makes the choice for
you (Go's formatter, Python has no arrow functions), the dimension does not
exist for it, and the table says why instead of shipping a detector that can
only ever count one side.

| Dimension | Python | Rust | Go |
| --- | --- | --- | --- |
| Import style | absolute package imports vs relative (`from .x import`) | `crate::` paths vs `super::` paths | none: `goimports` decides |
| Error construction | project exception classes vs bare `raise Exception(…)` | `?` propagation vs `.unwrap()` / `.expect()` outside tests | wrapping with `fmt.Errorf("…: %w", err)` vs returning `err` bare |
| Logging | `logging` / a logger vs `print()` in library modules | `tracing` / `log` macros vs `println!` / `eprintln!` in library code | `log/slog` (or zap, zerolog) vs `fmt.Println` / `log.Print*` |
| Guard clauses | early `return` / `raise` vs nested `if`/`else` | early `return`, `let … else` vs nested `if`/`else` | early `return` vs nested `if`/`else` |
| Type annotations | annotated vs unannotated function signatures | none: the compiler requires them | none: the compiler requires them |
| Decorators | project decorators replacing framework ones, as in TS | none (attributes are not a style choice) | none |

Each dimension gets a **scan analyzer** (statistical, per file) and, where a
before/after edit can show it, a **diff detector**. Type annotations get a
scan analyzer only: adding annotations is not a swap of one style for another.

---

## Step 1: Shared groundwork

- [x] Rule identity includes the language family for scan rules. Today a
      guard-clause rule from TypeScript and one from Python would share an
      instruction and therefore an id; give each language its own
      instruction text and scope (`**/*.py`, `**/*.rs`, `**/*.go`).
      *Landed as part of phase 7's manual gate:* `rule_id` now hashes the
      scope, so the same instruction under `**/*.py` and under the script
      scope are two ids whatever the wording. Each language still gets its own
      instruction text where the idiom differs (`raise`, `let … else`), and
      always for symmetric choices, because the opposite-rule check matches on
      instruction text across scopes. `Family::scope()` owns the four globs.
- [x] File roles for scopes per language: `test_*.py` / `*_test.py`,
      `*_test.go`, Rust `tests/` and `#[cfg(test)]` modules, so a scope like
      `**/*_test.go` is derived the way `*.service.ts` is today.
      (`ast/roles.rs`; a crate's `tests/` narrows to `<crate>/tests/**/*.rs`.
      `#[cfg(test)]` modules are inside a file, so step 3's analyzers and
      detectors skip them rather than a scope.)
- [x] Test-code exclusion: `.unwrap()` in tests and `print()` in scripts are
      not taste. Analyzers skip test files and test modules; Python skips
      `if __name__ == "__main__":` blocks and files under `scripts/`.
      (Path-based exclusion is in `roles::is_test_or_script` and the scan
      applies it before reading; TypeScript is unchanged. The in-file parts,
      `__main__` blocks and `#[cfg(test)]`, belong to steps 2 and 3.)
- [x] Contradiction skip-kinds and the scrubber's string-node kinds cover the
      three grammars (Python `string`, Rust `string_literal` / `raw_string_literal`,
      Go `interpreted_string_literal` / `raw_string_literal`). Go's content
      nodes (`interpreted_string_literal_content`, `raw_string_literal_content`)
      were the missing piece.
- [x] Bump `TALLY_FORMAT` once for the whole phase (`tally3`: counts split per
      language family, all phase 8 dimensions present).
- [x] **Structural-similarity deduplication (M12).** SPEC.md §2 promises
      "token-jaccard & structural similarity" deduplication; only the first
      exists. Two observations are also duplicates when their category and
      scope match and their avoid and prefer snippets have the same syntax
      shape (node kinds, identifiers normalised), whatever the instruction's
      wording. More languages mean more phrasings of the same preference, so
      this lands before them. Tests: same snippets in different words merge;
      same words with different snippets do not; snippets in two languages
      never merge. ("Identifiers normalised" means the ones carried through
      both halves of the edit; the identifiers the edit changed stay verbatim,
      or `@Entity → @AutoEntity` would merge with `@Index → @AutoIndex`.)
- [x] Tests: rule ids differ per language for the same dimension; role
      scopes for each language; test files are excluded.

## Step 2: Python

- [ ] Analyzers for the five Python dimensions in the table, as
      `ast/analyzers/python.rs`.
- [ ] Detectors: relative ↔ absolute import rewrite; `Exception` → project
      exception; `print` → logger; nested `if`/`else` → guard clause.
- [ ] Fixtures under `tests/fixtures/python/`, a positive and a negative per
      detector, in the style of the existing TypeScript fixtures.
- [ ] Turn Python on in `SupportedLanguage::has_analyzers`.
- [ ] **Precision gate:** scan three real open-source Python projects of
      different kinds (a web service, a library, a CLI). Every rule emitted is
      listed in this file with a yes/no from a human; any "no" is a bug to fix
      before the step is ticked.

## Step 3: Rust

- [ ] Analyzers for the four Rust dimensions, as `ast/analyzers/rust.rs`.
      `.unwrap()` on a `Mutex` lock or in `main` of a binary is common and
      defensible; count only library code outside `main` and tests.
- [ ] Detectors: `super::` ↔ `crate::` rewrite; `.unwrap()` → `?`;
      `println!` → `tracing`/`log`; nested `if`/`else` → guard clause or
      `let … else`.
- [ ] Fixtures under `tests/fixtures/rust/`.
- [ ] Turn Rust on in `has_analyzers`.
- [ ] **Precision gate:** three real Rust projects (a library, a CLI, a web
      service). OpenTaste itself may be one of them.

## Step 4: Go

- [ ] Analyzers for the three Go dimensions, as `ast/analyzers/go.rs`.
      `return err` directly after a call that already wraps is not "bare";
      count only returns of an `err` from another package's call.
- [ ] Detectors: `return err` → `fmt.Errorf("…: %w", err)`; `fmt.Println` →
      `slog`; nested `if`/`else` → guard clause.
- [ ] Fixtures under `tests/fixtures/go/`.
- [ ] Turn Go on in `has_analyzers`.
- [ ] **Precision gate:** three real Go projects.

## Step 5: The full test set

- [ ] Every analyzer: a unanimous file, a mixed file, and a file that must
      count nothing.
- [ ] Every detector: one positive and one negative fixture.
- [ ] Test code, scripts and `main` excluded where the step says so.
- [ ] A mixed-language repository (TS + Python + Go) produces separate rules
      per language and none that crosses languages.
- [ ] Commit learning (`learn --from-git-head`) and `learn-delta` both learn
      from a `.py`, `.rs` and `.go` correction.

---

## Definition of done

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean.
- [ ] The three precision gates recorded in this file, every emitted rule
      agreed with.
- [ ] The website's detector reference and language lists updated (phase 13
      checks this).
- [ ] V1-AUDIT M2 and M12 marked fixed.
- [ ] Status at the top of this file set to `completed`.
