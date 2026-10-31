# SPEC.md → code → phase: v1.0 traceability

**Checked:** 2026-10-30, against `dev` at `96def0a` (phases 1–7 merged).

Every claim in [SPEC.md](../SPEC.md), section by section. **Shipped** means the
code exists and was checked, with where. A phase reference means a
`plans/PHASE-*.md` step builds or fixes it. Nothing in SPEC.md is left without
one or the other.

Phase 13 keeps this file current: when a phase lands, its rows change from a
phase reference to **Shipped**.

## Header and §1: overview

| Claim | Status |
| --- | --- |
| Works with Cursor, Claude Code, Windsurf, VS Code Copilot, Zed | **Shipped**: five sync targets (`config::Paths`). Zed via `AGENTS.md`; phase 13 step 2 confirms Zed's current rules-file behaviour and states it in SPEC.md |
| MIT, offline, zero telemetry | **Shipped**: no network crate in the dependency tree; no telemetry code |
| Three loops: scan, delta learner, MCP | **Shipped**: `ast/scan.rs`, `taste learn-delta` + extension, `taste-mcp` |
| Scanner reads "recent Git history" | **Shipped** (phase 10): `taste scan --history N`, and `taste init` with a 200-commit / 180-day window; `taste-cli/src/commands/history.rs` |
| tree-sitter + rayon | **Shipped**: `ast/parser.rs`, `par_iter` in `ast/scan.rs` |
| Scan timings and "<300ms" | **Shipped** (phase 12 step 3): 1,000 files cold in 282–294 ms on four cores, measured by `taste-core/tests/scan_bench.rs`; SPEC.md §1 states the measured numbers |
| Rules at ≥ 0.80 are synced to the five targets | **Shipped**: `TasteRule::is_active`, `transpiler/` |

## §2: directory structure

| Claim | Status |
| --- | --- |
| `config.rs` paths for `rules.json`, `cache.db` | **Shipped** |
| `config.rs` / `sqlite_store.rs`: `~/.taste/global.db` | Phase 11 (M6) |
| `models.rs` types | **Shipped** |
| `parser.rs`: TS/JS, Python, Rust, Go | **Shipped**: parsing, and analyzers and detectors for all four (phase 8, `ast/analyzers/{python,rust,go}.rs`, `diff/{python,rust,go}.rs`) |
| `imports.rs`: alias vs relative, named vs default | **Shipped**; barrel exports fixed in phase 7 |
| `types.rs`: interface vs type | **Shipped** |
| `types.rs`: strict null checks | **Shipped** (phase 9 step 1): read from `tsconfig.json`, `ast/tsconfig.rs`; diff detector in `diff/structural.rs` |
| `types.rs`: zod schemas | **Shipped** (phase 9 step 2): `ast/analyzers/validation.rs` |
| `functions.rs`: arrow vs declaration, guard clauses | **Shipped** |
| `arch.rs`: custom decorators, custom error classes | **Shipped** |
| `arch.rs`: DI patterns | **Shipped** (phase 9 step 3): `ast/analyzers/di.rs`, NestJS and Angular only |
| `diff/structural.rs` with `similar` + node matching | **Shipped** |
| `diff/scrubber.rs` | **Shipped**; boundary fix in phase 7 |
| `scorer/confidence.rs` | **Shipped** |
| `scorer/dedup.rs`: token-jaccard | **Shipped** |
| `scorer/dedup.rs`: structural similarity | **Shipped** (phase 8 step 1): `scorer/dedup.rs::same_edit` |
| `storage/json_store.rs`, deterministic | **Shipped** |
| `transpiler/`: marker, Cursor `.mdc`, `CLAUDE.md`, `.windsurfrules`, `AGENTS.md` + Copilot | **Shipped** |
| `taste-mcp`: zero-dependency JSON-RPC 2.0 over stdio | **Shipped**: depends only on `taste-core`, `serde`, `serde_json`, `anyhow` |
| `tools.rs` names | **Shipped** as `get_coding_taste`, `record_coding_taste`, `reject_coding_taste`; §2's `reject_taste` is fixed in phase 13 step 2 (D2) |
| `taste-cli` commands: init, scan, sync, learn, status, watch | **Shipped** |
| `scan.rs`: "repo & git log analysis" | Repo **shipped**; git log in phase 10 |
| `tui/app.rs`: list, filter, toggle, delete | **Shipped** |
| `tui/app.rs`: edit | **Shipped** (phase 12 step 1): `[e]`, saved through `RuleStore::update_rule` |
| Extension: `extension.ts`, `deltaTracker.ts`, `binaryRunner.ts`, `statusBar.ts` | **Shipped** |
| npm wrapper `bin/taste.js` | **Shipped** as `@opentaste/cli` |
| `npx opentaste` | Cannot be built as written (name taken); decided in phase 13 step 4 (M11) |

## §3: workspace configuration

| Claim | Status |
| --- | --- |
| Workspace members, MIT, repository URL | **Shipped** |
| Dependency versions | Drifted (tree-sitter 0.27, ratatui 0.30, notify 8, MSRV 1.90); phase 13 step 2 (D3) |

## §4: data models and storage

| Claim | Status |
| --- | --- |
| `rules.json` shape, sorted by id, `$schema` | **Shipped**; schema served from GitHub (PR #9) |
| `detected_stack` like `typescript, nestjs, react, postgresql` | **Shipped** in phase 7 step 5 |
| `RuleCategory`, `RuleSource`, `TasteRule`, `RulesFile`, `ProjectContext` | **Shipped** |
| SQLite `candidate_rules`, `file_snapshots` | **Shipped** (plus `rejected_rules`, `learned_changes`, `pending_deltas`) |
| Same schema for `global.db` | Phase 11 step 2 |

## §5: core engine

| Claim | Status |
| --- | --- |
| Starting confidences 0.45 / 0.95 / scan ratio ≥ 0.80 over ≥ 10 files | **Shipped** |
| Reinforcement `+0.20 × (1 − c)`, ceiling 0.99 | **Shipped** |
| Promotion at ≥ 0.80 and ≥ 3 sightings (or chat) | **Shipped** |
| Promotion triggers `taste sync` | **Shipped** in `learn-delta`, the hook, MCP, `init`, `watch` and, since phase 10 step 4 (W13), a standalone `taste scan` |
| Contradiction decay `× 0.75` | **Shipped** |
| Demotion below 0.50 | **Shipped** in phase 7 step 1 (was unreachable) |
| §5.2 detectors 1–7, including control-flow flattening | **Shipped** (AGENTS.md still lists control flow as deferred: phase 13 step 3, D6) |
| §5.3 entropy and pattern scrubbing | **Shipped** |

## §6: transpiler

| Claim | Status |
| --- | --- |
| Cursor `.mdc` grouped by category, `globs`, `alwaysApply: false` | **Shipped** |
| Marker-block injection preserving developer content | **Shipped** |

## §7: MCP server

| Claim | Status |
| --- | --- |
| `get_coding_taste` returns active matching rules | **Shipped** |
| `record_coding_taste` saves at 0.95, writes `rules.json`, syncs at once | **Shipped**; snippets scrubbed since PR #10 |
| `reject_coding_taste` disables or deletes | **Shipped** |

## §8: CLI and dashboard

| Claim | Status |
| --- | --- |
| `taste init` steps 1–5 | **Shipped** |
| `taste scan` with `WalkParallel` | Scan **shipped**; parallel walk in phase 12 step 3 |
| `taste sync`, `learn-delta`, `status`, `ui` | **Shipped** |
| Dashboard 45/55 panes, confidence bar, category badge, keybindings | **Shipped** |
| Syntax-highlighted avoid/prefer box | **Shipped** (phase 12 step 2): `tui/highlight.rs` |

## §9: extension

| Claim | Status |
| --- | --- |
| Runs in VS Code, Cursor, Windsurf | **Shipped**; checked in each editor in phase 14 step 5 |
| Published to the Marketplace and Open VSX | Phase 14 |
| Burst of ≥ 4 lines opens a window | **Shipped** |
| Correction within the window, 180 s, on save | **Shipped**; size rule settled in phase 7 step 4 |
| `learn-delta` over stdin, toast with Keep Rule / Ignore | **Shipped** |

## §10: implementation plan

| Claim | Status |
| --- | --- |
| Phases 1–5 | **Shipped** |
| Phase 6 | Code **shipped**; its two release-time gates moved to phase 14 |

## Findings from this check

Checking every SPEC.md line turned up two gaps the audit had missed, both
now in V1-AUDIT.md: **M12**, structural-similarity deduplication (phase 8
step 1), and **W13**, a standalone `taste scan` that confirms rules without
running `taste sync` (phase 10 step 4). It also confirmed that the AGENTS.md *not deferred* table
still lists the control-flow detector, which already ships (D6, phase 13).
