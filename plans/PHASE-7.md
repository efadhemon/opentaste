# Phase 7: Correctness of the learning pipeline

**Status:** ongoing

| | |
| --- | --- |
| **Blocked by** | Nothing. Phase 6's code is done; its open gates are release steps. |
| **Blocks** | Phases 8 to 10, which feed more observations into the same pipeline. |
| **Rough size** | 2–3 sessions |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) W3–W12 and M10 |

Phases 8 to 10 add languages, detectors and history. Every one of them sends
more observations through `record_observation`, scoring, contradiction and
sync. Anything those layers get wrong today gets multiplied, so this phase
fixes them first.

---

## Step 1: Make demotion reachable (W3)

A confirmed rule leaves the editor files the moment it falls below 0.80, but
`contradictions()` only looks at *active* rules, so it is never contradicted
again and never reaches the 0.50 demotion SPEC.md §5.1 describes. It stays in
`rules.json` forever.

- [x] `contradictions()` checks every **enabled confirmed** rule whose scope
      matches the file, active or not, through the new
      `RuleStore::enabled_rules_for_file`. A disabled rule is the developer's
      choice and is left alone.
- [x] A rule that keeps being contradicted therefore decays 0.82 → 0.62 →
      0.47 (or 0.95 → 0.71 → 0.53 → 0.40 from chat) and is moved to the
      candidate cache by the existing `apply_contradiction` path.
- [x] Tests in `tests/promotion_flow.rs` drive the real detection, not
      `apply_contradiction` directly (the old tests did, which is why the
      unreachable path went unnoticed): three contradicting edits take a
      chat rule out of the editor files, then down, then into the candidates;
      a disabled rule is never decayed. The first fails without the fix.

## Step 2: Decorator contradictions (W4)

An anti-pattern like `@Get()` is under the 8-character needle, so rules from
`DecoratorSwap` never decay.

- [x] When a rule's anti-pattern is a single decorator, match decorators by
      name on the syntax tree instead of by substring: count `decorator`
      nodes whose name equals the anti-pattern's, before and after. This
      covers every decorator anti-pattern, short or long, so `@Entity("users")`
      is now contradicted by `@Entity("orders")` too: the preference is about
      the decorator, not its arguments.
- [x] Tests: adding `@Entity("orders")` contradicts a rule whose
      anti-pattern is `@Entity()` (fails without the change); a decorator
      only mentioned in a comment or string does not; an unchanged legacy
      decorator does not; `@EntityRepository` does not match `@Entity`; and
      unit tests for reading the decorator name.

## Step 3: One correction, one count, across sources (W5)

The extension learns a correction when the file is saved. If the AI's block
had already been committed, the next commit carries the same correction and
the post-commit hook learns it again.

- [x] Reproduced with the debug build: a correction learned by `learn-delta`
      (confidence 0.45, seen once), then committed, was learned again by
      `learn --from-git-head` (0.56, seen twice). The detector extracts the
      same snippets from the extension's block and from the committed file,
      so the two can be matched exactly.
- [x] `learn-delta` remembers each observation in a `pending_deltas` cache
      table (schema version 4), keyed by file, category, instruction and
      both snippets (`commands::delta_key`). `learn --from-git-head` skips an
      observation whose key is there, once, and removes it. Keys older than
      30 days are pruned. The `learn-delta` stdout contract is unchanged.
- [x] Tests: the reproduction counts once (fails without the skip); a
      different correction of the same kind in the same file still counts a
      second time; a remembered key is taken at most once and stale ones are
      pruned.

## Step 4: Settle the correction heuristic (W6)

SPEC.md §9 says a correction is a "1–15 char" edit inside the AI block. The
extension treats any edit that is not itself a burst as a correction.

- [x] Decided with evidence. Measured as single editor edits, three of
      seven realistic corrections fit a 15-character cap (one key at a time,
      a rename to `NotFoundError` at 13, typing `Auto`) and four do not: a
      pasted alias import path (17), a pasted `this.logger.log(` (16), a
      "convert to arrow function" code action (41) and an
      interface-to-type header (20). Those four are exactly the
      transformations the detectors learn, so a cap would mostly lose real
      corrections, while precision already comes from the detectors.
      **Outcome: no size limit; any non-burst edit inside the block counts.**
- [x] SPEC.md §9 now says so and gives the numbers; the reason is in the
      `deltaTracker.ts` header comment; a new extension test pins it with a
      41-character code-action edit.

## Step 5: Monorepo scopes and stack detection (W7, M10)

- [x] A learned scope starts at the nearest directory under the project root
      that has its own `package.json`, `Cargo.toml`, `pyproject.toml` or
      `go.mod` (`config::PACKAGE_MANIFESTS`, `Paths::package_dir`):
      `apps/api/src/x.service.ts` gives `apps/api/**/*.service.ts`. Without
      one, behaviour is unchanged. Both learners call the new
      `taste_core::analyze_in`; `analyze` itself stays pure and path-only.
- [x] `detected_stack` adds `typescript` (a `tsconfig.json` or the
      `typescript` dependency), databases (`postgresql`, `mysql`, `mongodb`,
      `sqlite`, `prisma`, `typeorm`, `drizzle`), and reads every workspace
      package from `workspaces` in `package.json` (array or `packages`
      object) and the `packages:` list of `pnpm-workspace.yaml`, including
      `!` exclusions, four levels deep, never inside `node_modules`.
- [x] Tests: package scope with a `package.json` and with a `go.mod`; no
      scope change without a manifest, including a folder that only looks
      like a package; a CLI test that `learn-delta` uses it (fails if it
      calls `analyze`); an npm workspace whose frameworks are only in
      `apps/*/package.json`; a pnpm workspace with an exclusion;
      `typescript`, `prisma` and `mysql` in a single package; packages
      outside the workspace globs and inside `node_modules` ignored.

## Step 6: Barrel exports (W8)

- [x] `export * from`, `export { x } from` (anything with a `source`) and
      `export type { … }` no longer count toward named exports.
      `export type X = …` and `export interface` are declarations and still
      count.
- [x] Cached tallies carry a format tag, now `tally2`. Without the bump, a
      file whose snapshot was written by an older build would keep its old
      count until it changed; with it, the next scan recounts every file.
      Bump `TALLY_FORMAT` whenever an analyzer counts differently.
- [x] Tests: a barrel file contributes nothing to either side; a module with
      four declarations, a re-export, a type-only list and a default counts
      4 named and 1 default (both fail without the change); a tally in the
      old format is not reused.

## Step 7: Scrubber boundary (W9)

- [ ] Vendor-prefix patterns (`AKIA`, `ghp_`, `sk_live_`, …) match after any
      non-alphanumeric character or at the start, not only at a `\b`, for
      prefixes that cannot begin an English word.
- [ ] Tests: `xxxAKIA…` and `token=ghp_…` are redacted; the English words in
      the existing negative fixtures still are not.

## Step 8: Scan hygiene (W10, W11, W12)

- [ ] Snapshot rows for files the walk no longer sees are deleted at the end
      of a scan.
- [ ] Files in a language with no analyzers are skipped before they are read.
      The check goes through one function, so phase 8 turns languages on in
      one place.
- [ ] The `MAX_FILE_BYTES` doc comment matches the `>` comparison.
- [ ] Tests: deleting a file removes its snapshot after the next scan; a Rust
      file is not read or snapshotted while Rust has no analyzers.

## Step 9: The full test set

The complete list this phase cannot ship without. Each was written with its
step.

- [ ] Demotion through two contradictions; disabled rules untouched.
- [ ] Decorator contradiction by name, and its three negatives.
- [ ] Cross-source counting: once for the same correction, twice for two.
- [ ] Monorepo scope, workspace stack, `typescript` and database detection.
- [ ] Barrel files count nothing.
- [ ] Glued vendor keys redacted.
- [ ] Snapshot pruning; unsupported languages not read.

---

## Definition of done

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean.
- [ ] `pnpm -r run typecheck` and `pnpm -r run test` green if step 4 touched
      the extension.
- [ ] SPEC.md §5.1 and §9 match the code.
- [ ] **Manual gate:** run `taste init` on a real monorepo, replay its last
      20 commits through `taste learn --from-git-head`, and confirm every
      rule's scope names the right package and nothing was counted twice.
- [ ] The W3–W12 and M10 rows in V1-AUDIT.md marked fixed.
- [ ] Status at the top of this file set to `completed`.
