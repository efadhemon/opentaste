# v1.0 audit: what SPEC.md promised, what shipped, what is missing

**Status:** ongoing

**Audited:** 2026-10-30, against `dev` at `b2ddabc` plus `fix/learning-safety`
(PR #10). Every row below was checked in the code, not taken from a plan
file's tick box.

**Decision (2026-10-30):** features are not cut. Everything SPEC.md promises,
including the rows AGENTS.md listed as *deferred past v0.1*, ships before the
first public release, which becomes **v1.0.0**. The work is ordered into
phases 7 to 12 at the end of this file.

The rule from AGENTS.md still applies to every new detector: **a wrong rule is
worse than a missing rule.** "Not cut" means each feature is built and held to
the same precision gate as the existing detectors, not that a detector ships
before it is reliable.

---

## 1. Promised in SPEC.md and not built

| # | SPEC says | What exists | Where |
| --- | --- | --- | --- |
| M1 | Scanner learns from "existing source files **and recent Git history**" (§1) | Only the working tree is scanned. `taste learn --from-git-head` sees the last commit only. | `ast/scan.rs`, `commands/learn.rs` |
| M2 | Parser and analyzers for **TS/JS, Python, Rust, Go** (§2, §10 phase 3) | All four grammars parse, but every analyzer and every diff detector returns nothing unless `is_script()`. Python, Rust and Go teach nothing. | `ast/analyzers/mod.rs:189`, `diff/structural.rs:64` **Fixed in phase 8 steps 2–4.** |
| M3 | `types.rs`: **strict null checks** | Not detected | `ast/analyzers/types.rs` **Fixed in phase 9 step 1.** |
| M4 | `types.rs`: **zod schemas** | Not detected | `ast/analyzers/types.rs` **Fixed in phase 9 step 2.** |
| M5 | `arch.rs`: **DI patterns** | Not detected | `ast/analyzers/arch.rs` |
| M6 | **`~/.taste/global.db`**, cross-project rules (§2, §4.3) | `Paths::global_db()` exists; nothing reads or writes the file | `config.rs:90` |
| M7 | Terminal dashboard can **edit** a rule (§2 `app.rs`: "toggle, edit, delete") | Toggle, promote, delete, sync, filter. No edit. | `tui/app.rs` |
| M8 | Dashboard shows a **syntax-highlighted** avoid/prefer diff box (§8) | Red/green colouring, no syntax highlighting | `tui/ui.rs` |
| M9 | Scanner walks with **`ignore::WalkParallel`** and meets **<300 ms** (§1, §8) | Serial `WalkBuilder` walk; only parsing is parallel. 991 files took 321 ms in release. | `ast/scan.rs:65` |
| M10 | `detected_stack` like `["typescript", "nestjs", "react", "postgresql"]` (§4.1) | Detects rust, python, go and some JS frameworks. Never `typescript` and never a database. Root `package.json` only. | `ast/scan.rs:255` **Fixed in phase 7 step 5.** |
| M12 | Deduplication by "token-jaccard **& structural similarity**" (§2 `scorer/dedup.rs`) | Token overlap of the instruction plus scope equality only. Two observations with different wording but the same avoid/prefer snippets become two rules. Found while validating the phase plans. | `scorer/dedup.rs` **Fixed in phase 8 step 1.** |
| M11 | `npx opentaste` runs the tool (§2, §10 phase 6) | The npm name `opentaste` belongs to an unrelated package. The wrapper is `@opentaste/cli`. **Cannot be built as written**; see section 4. | `packages/npm-opentaste` |

## 2. Built, but wrong or not working as described

| # | Problem | Effect | Found |
| --- | --- | --- | --- |
| W1 | `taste learn --from-git-head` counted the same change on every run (`--amend`, cherry-pick, a manual rerun) | One correction could be pushed most of the way to a confirmed rule | Website work. **Fixed in PR #10.** |
| W2 | `record_coding_taste` saved snippets unscrubbed into the git-tracked `rules.json` | A key pasted in chat could be committed | Website work. **Fixed in PR #10.** |
| W3 | Demotion to a candidate below 0.50 (§5.1) can never happen. Contradictions are only checked for *active* rules (≥ 0.80), and one contradiction takes any rule below 0.80, so a rule is never contradicted a second time. | Contradicted rules stay in `rules.json` forever, inactive, instead of returning to the candidate cache | This audit. **Fixed in phase 7 step 1.** |
| W4 | Decorator anti-patterns such as `@Get()` are shorter than the 8-character contradiction needle | Rules learned from decorator swaps never decay | Phase 3 review, improvement 8. **Fixed in phase 7 step 2.** |
| W5 | The extension learns a correction on save, and the post-commit hook may learn the same correction again from the commit | Possible double count across the two sources when AI code is committed before it is corrected | This audit; needs a reproduction. **Fixed in phase 7 step 3 (reproduced: 0.45 → 0.56 for one correction).** |
| W6 | §9 says a correction is a "1–15 char" edit inside the block. The extension treats *any* non-burst edit inside it as a correction. | A deviation, not necessarily a bug. Decide which is right and make the spec and the code agree. | This audit. **Fixed in phase 7 step 4 (no size limit; SPEC.md updated).** |
| W7 | Monorepo scopes: `apps/api/src/x.service.ts` gives `apps/**/*.service.ts`, covering every app | A rule learned in one app applies to all of them | Phase 3 review, improvement 6. **Fixed in phase 7 step 5.** |
| W8 | Re-export barrels and `export type {…}` count as named exports | Inflates the named-export ratio in barrel-heavy projects | Phase 3 review, improvement 10. **Fixed in phase 7 step 6.** |
| W9 | Vendor key patterns need a word boundary, so a key glued to preceding text (`xxxAKIA…`) is not matched | A secret can slip past the scrubber | Phase 3 review, improvement 12. **Fixed in phase 7 step 7.** |
| W10 | Snapshot rows for deleted files are never pruned | The cache grows; stale tallies | Phase 3 review, improvement 5. **Fixed in phase 7 step 8.** |
| W11 | Non-script files are read, hashed, parsed and snapshotted for no counts | Wasted scan time (until M2 lands) | Phase 3 review, improvement 3. **Fixed in phase 7 step 8.** |
| W12 | `MAX_FILE_BYTES` doc says "at or above", the code uses `>` | Doc mismatch | Phase 3 review, improvement 9. **Fixed in phase 7 step 8.** |

| W13 | A standalone `taste scan` confirms rules but does not run `taste sync`, unlike every other learner (§5.1: promotion "triggers `taste sync`") | Rules sit in `rules.json` without reaching the editor files until something else syncs | Plan validation. Phase 10 step 4. |

## 3. Documentation that is out of date

| # | Document | Problem |
| --- | --- | --- |
| D1 | `README.md` | Says the AST scanner, MCP server, CLI, dashboard and extension are "not yet implemented". All of them are. |
| D2 | `SPEC.md` §2, §10 | `npx opentaste` (see M11); MCP tool called `reject_taste` in §2 and `reject_coding_taste` in §7 (the code uses the second) |
| D3 | `SPEC.md` §3 | Dependency versions are the pre-phase-3 ones (`tree-sitter 0.24`; the workspace uses 0.27 and raised the MSRV to 1.90 for it) |
| D4 | `SPEC.md` §5.1 | Describes demotion below 0.50 as reachable; see W3 |
| D5 | `SPEC.md` §9 | Correction heuristic; see W6 **Fixed in phase 7 step 4.** |
| D6 | `AGENTS.md` | The *Deferred past v0.1* table contradicts the decision above. It now points here. |

## 4. Cannot be done as written

**`npx opentaste`.** The name was registered by an unrelated project in
February 2026. The options are: keep `@opentaste/cli` (`npx @opentaste/cli`,
and `taste` after a global install), or ask npm support for the name under
their dispute policy. Recommended: keep `@opentaste/cli`, fix SPEC.md, and
optionally file the request in parallel.

## 5. Still open from phase 6 (release gates, the user's)

- The manual check in VS Code, Cursor and Windsurf.
- `npx @opentaste/cli status` on a machine with no Rust toolchain.
- Publishing: the `@opentaste` npm org, the `opentaste` Marketplace
  publisher, the Open VSX namespace, and the three secrets in the `release`
  environment (`NPM_TOKEN` is done).

---

## Phases to v1.0.0

Each phase gets its own `plans/PHASE-N.md` with steps, tests and a definition
of done before work starts, in the same format as phases 3 to 6. Every step
ships with its tests, and every new detector also needs the manual precision
gate: run on real repositories, and a human agrees with every rule it emits.

| Phase | Scope | Rows |
| --- | --- | --- |
| **7: Correctness** | Make demotion reachable (W3); decorator contradictions (W4); reproduce and fix cross-source double counting (W5); settle the correction heuristic (W6); monorepo scopes and stack detection (W7, M10); barrel exports (W8); scrubber boundary (W9); snapshot pruning and skipping non-script files (W10, W11, W12) | W3–W12, M10 |
| **8: More languages** | Analyzers and diff detectors for Python, Rust and Go, each with its own fixtures and precision gate (M2); structural-similarity deduplication (M12) | M2, M12 |
| **9: Missing TypeScript detectors** | Strict null checks from `tsconfig.json` (M3), zod schemas (M4), dependency-injection patterns (M5) | M3–M5 |
| **10: Git history** | Mine recent history on `taste scan`, with a bounded window and the W1 fingerprints so nothing is counted twice (M1) | M1 |
| **11: Cross-project rules** | `~/.taste/global.db`: what is shared, opt-in, and how a project overrides it (M6). Needs a privacy design before code. | M6 |
| **12: Dashboard and speed** | Edit a rule in the dashboard (M7), syntax highlighting (M8), parallel walk and the <300 ms target (M9) | M7–M9 |
| **13: Docs** | README, SPEC.md and AGENTS.md brought in line (D1–D6, M11); the website's remaining steps | D1–D6, M11 |
| **14: Release** | Section 5, then tag `v1.0.0` on `main` | — |

Each phase has its own file, [PHASE-8.md](PHASE-8.md) to
[PHASE-14.md](PHASE-14.md). [V1-TRACEABILITY.md](V1-TRACEABILITY.md) maps
every claim in SPEC.md to the code that implements it or the phase step
that will.

Phase 7 comes first because phases 8 to 10 multiply whatever the learning
pipeline does wrong: more languages and more history feed more observations
into the same scoring path.
