# OpenTaste (`taste`) — Master Architecture & Implementation Specification

> **Tagline:** The 100% Local-First, Zero-Cloud Universal "Taste Mode" Engine for AI Coding Editors (Cursor, Claude Code, Windsurf, VS Code Copilot, Zed).
> **Architecture:** Hybrid Rust Core (80%) + TypeScript IDE Extension & NPM Wrapper (20%)
> **License:** MIT | **Privacy:** 100% Offline Execution, Zero Telemetry, No External Server.

---

## 1. System Overview & Core Value Proposition

**OpenTaste** passively learns a developer's and project's coding style, architectural patterns, and conventions ("Coding Taste") without requiring manual rule writing or cloud LLMs.

It operates through three local intelligence loops:

1. **Cold-Start AST Repo Scanner (`taste scan`):** Parses existing source files and recent Git history using native `tree-sitter` and `rayon` multi-threading to extract statistical codebase norms. History is a bounded window: `taste init` reads the last 200 non-merge commits on the first-parent chain or 180 days, whichever is fewer, and `taste scan --history N` reads the last `N`; both start after the newest commit already read. Commits by bots, reverts and sweeps touching over 50 files are skipped. History records corrections only, never contradictions, counts each change once (the post-commit hook shares its fingerprints), and leaves out a past correction towards one side of a symmetric choice when the code today mostly takes the other. 200 commits read in under a second on the repositories measured in `plans/PHASE-10.md`. A cold scan of 1,000 files (700 TypeScript, 100 each of Python, Rust and Go) takes 282–294 ms in a release build on four cores, and a warm scan with nothing changed 12–14 ms; the benchmark is `taste-core/tests/scan_bench.rs` and the numbers are recorded in `plans/PHASE-12.md`. Larger trees take proportionally longer.
2. **Post-AI Structural Delta Learner (`taste learn-delta` + IDE Extension):** Detects when an AI agent generates code and the human developer subsequently edits/fixes that exact code block. It computes the AST diff (`AI Output -> Human Correction`) and extracts a candidate taste rule.
3. **Host-Agent MCP Piggybacking (`taste mcp`):** Runs a lightweight `stdio` Model Context Protocol (MCP) server so the user's existing AI editor (Cursor/Claude) automatically saves explicit preferences into the local database whenever corrected in chat.

Once rules reach high confidence (`>= 0.80`), the **Universal Transpiler (`taste sync`)** compiles them non-destructively into:

- `.cursor/rules/opentaste-*.mdc` (File-scoped via `globs`)
- `CLAUDE.md` (Marker-block injected)
- `.windsurfrules` (Marker-block injected)
- `AGENTS.md` & `.github/copilot-instructions.md` (Marker-block injected)

---

## 2. Monorepo Directory Structure

```text
opentaste/
├── Cargo.toml                        # Rust Workspace Root
├── pnpm-workspace.yaml               # TypeScript Workspace Root
├── package.json                      # Root scripts & dev tooling
├── README.md
├── LICENSE                           # MIT
│
├── crates/
│   ├── taste-core/                   # [Rust Lib] Core Engine (AST, Diff, Scorer, Storage, Transpiler)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── config.rs             # Paths (.taste/rules.json, .taste/cache.db, ~/.taste/global.db)
│   │       ├── models.rs             # TasteRule, CandidateRule, RuleCategory, ProjectContext
│   │       ├── ast/
│   │       │   ├── mod.rs
│   │       │   ├── parser.rs         # Multi-language tree-sitter wrapper (TS/JS, Python, Rust, Go)
│   │       │   └── analyzers/        # Statistical pattern detectors
│   │       │       ├── imports.rs    # Alias (@/) vs relative (../../), named vs default exports
│   │       │       ├── types.rs      # interface vs type, strict null checks, zod schemas
│   │       │       ├── functions.rs  # arrow vs function decl, early return guard clauses
│   │       │       └── arch.rs       # Custom decorators (@AutoEntity), custom Error classes, DI patterns
│   │       ├── diff/
│   │       │   ├── mod.rs
│   │       │   ├── structural.rs     # AST diffing using `similar` + tree-sitter node matching
│   │       │   └── scrubber.rs       # Regex + Entropy secret/API-key redaction before saving snippets
│   │       ├── scorer/
│   │       │   ├── mod.rs
│   │       │   ├── confidence.rs     # Weighted scoring, promotion (occurrences >= 3), exponential decay
│   │       │   └── dedup.rs          # Token-jaccard & structural similarity deduplication
│   │       ├── storage/
│   │       │   ├── mod.rs
│   │       │   ├── json_store.rs     # Deterministic read/write for Git-tracked `.taste/rules.json`
│   │       │   └── sqlite_store.rs   # `rusqlite` handler for `.taste/cache.db` & `~/.taste/global.db`
│   │       └── transpiler/
│   │           ├── mod.rs
│   │           ├── marker.rs         # Non-destructive `<!-- OPENTASTE:START -->` block injector
│   │           ├── cursor_mdc.rs     # Generates scoped `.cursor/rules/opentaste-<category>.mdc`
│   │           ├── claude_md.rs      # Updates `CLAUDE.md`
│   │           ├── windsurf.rs       # Updates `.windsurfrules`
│   │           └── agents_md.rs      # Updates `AGENTS.md` & `.github/copilot-instructions.md`
│   │
│   ├── taste-mcp/                    # [Rust Lib] Zero-dependency JSON-RPC 2.0 Stdio MCP Server
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── protocol.rs           # JSON-RPC request/response types
│   │       ├── server.rs             # Stdio loop handling `initialize`, `tools/list`, `tools/call`
│   │       └── tools.rs              # Implements `get_coding_taste`, `record_coding_taste`, `reject_taste`
│   │
│   └── taste-cli/                    # [Rust Binary: `taste`] CLI, Watcher, Git Hooks & TUI
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs               # Entry point using `clap`
│           ├── commands/
│           │   ├── init.rs           # Creates `.taste/`, installs Git hooks, registers `.cursor/mcp.json`
│           │   ├── scan.rs           # Runs cold-start repo & git log analysis
│           │   ├── sync.rs           # Runs Universal Transpiler across all editor targets
│           │   ├── learn.rs          # Processes stdin diffs or git commit deltas
│           │   ├── status.rs         # Prints quick CLI summary table
│           │   └── watch.rs          # Background file watcher using `notify` crate
│           └── tui/
│               ├── mod.rs
│               ├── app.rs            # `ratatui` state machine (Rule list, filter, toggle, edit, delete)
│               └── ui.rs             # Terminal layout renderer
│
├── packages/
│   ├── vscode-opentaste/             # [TypeScript] Universal VS Code / Cursor / Windsurf Extension
│   │   ├── package.json
│   │   ├── tsconfig.json
│   │   └── src/
│   │       ├── extension.ts          # Extension lifecycle & command registration
│   │       ├── deltaTracker.ts       # Detects AI burst insertions vs subsequent human edits
│   │       ├── binaryRunner.ts       # Spawns local `taste` binary via child_process
│   │       └── statusBar.ts          # IDE status bar item & non-intrusive learning toast notifications
│   │
│   └── npm-opentaste/                # [TypeScript/Node] Cross-platform `npx opentaste` installer wrapper
│       ├── package.json
│       └── bin/
│           └── taste.js              # Resolves OS/Arch and executes the prebuilt Rust binary
```

---

## 3. Workspace Configurations & Dependencies

### 3.1 Root `Cargo.toml`

```toml
[workspace]
resolver = "2"
members = [
    "crates/taste-core",
    "crates/taste-mcp",
    "crates/taste-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
authors = ["OpenTaste Contributors"]
repository = "https://github.com/efadhemon/opentaste"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
anyhow = "1.0"
thiserror = "1.0"
chrono = { version = "0.4", features = ["serde"] }
uuid = { version = "1.10", features = ["v4", "serde"] }
sha2 = "0.10"
ignore = "0.4"
rayon = "1.10"
globset = "0.4"
similar = { version = "2.6", features = ["inline"] }
rusqlite = { version = "0.32", features = ["bundled"] }
tree-sitter = "0.24"
tree-sitter-typescript = "0.23"
tree-sitter-python = "0.23"
tree-sitter-rust = "0.23"
clap = { version = "4.5", features = ["derive"] }
notify = "6.1"
ratatui = "0.28"
crossterm = "0.28"
regex = "1.11"
dirs = "5.0"
```

---

## 4. Data Models & Storage Architecture

### 4.1 Git-Tracked Project Schema (`.taste/rules.json`)

Sorted deterministically by `id` to prevent Git merge conflicts.

```json
{
  "$schema": "https://raw.githubusercontent.com/efadhemon/opentaste/main/schema/v1.json",
  "version": "1.0.0",
  "project_context": {
    "detected_stack": ["typescript", "nestjs", "react", "postgresql"],
    "total_files_scanned": 342,
    "last_updated": "2026-10-29T00:00:00Z"
  },
  "rules": [
    {
      "id": "arch-custom-entity-decorators",
      "category": "architecture",
      "scope": "src/**/*.entity.ts",
      "confidence": 0.95,
      "occurrences": 14,
      "enabled": true,
      "source": "ast-diff",
      "instruction": "Use custom @AutoEntity() and @AutoIndex() decorators instead of raw TypeORM @Entity() and @Index().",
      "anti_pattern": "@Entity('users')\nexport class User {}",
      "preferred_pattern": "@AutoEntity()\nexport class User {}",
      "updated_at": "2026-10-29T00:00:00Z"
    }
  ]
}
```

### 4.2 Rust Core Structs (`crates/taste-core/src/models.rs`)

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum RuleCategory {
    Architecture,
    CodeStyle,
    Imports,
    Types,
    ErrorHandling,
    Naming,
    Testing,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RuleSource {
    AstScan,
    AstDiff,
    McpFeedback,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TasteRule {
    pub id: String,
    pub category: RuleCategory,
    pub scope: String, // Glob pattern e.g., "src/**/*.ts" or "**/*"
    pub confidence: f32, // 0.0 to 1.0
    pub occurrences: u32,
    pub enabled: bool,
    pub source: RuleSource,
    pub instruction: String,
    pub anti_pattern: Option<String>,
    pub preferred_pattern: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesFile {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub version: String,
    pub project_context: ProjectContext,
    pub rules: Vec<TasteRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectContext {
    pub detected_stack: Vec<String>,
    pub total_files_scanned: usize,
    pub last_updated: DateTime<Utc>,
}
```

### 4.3 Local SQLite Schema (`.taste/cache.db` & `~/.taste/global.db`)

Used for tracking unpromoted candidate patterns (`confidence < 0.80`) and file AST snapshots without polluting Git:

```sql
CREATE TABLE IF NOT EXISTS candidate_rules (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    scope TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.35,
    occurrences INTEGER NOT NULL DEFAULT 1,
    contradictions INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL,
    instruction TEXT NOT NULL,
    anti_pattern TEXT,
    preferred_pattern TEXT,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS file_snapshots (
    file_path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    ast_signature TEXT NOT NULL,
    captured_at TEXT NOT NULL
);
```

---

## 5. Core Engine Mechanics (`crates/taste-core`)

### 5.1 Confidence Scoring & Promotion Math (`scorer/confidence.rs`)

- **Initial Candidate Creation:**
  - From `AstDiff` (Human editing AI output): starts at `occurrences = 1`, `confidence = 0.45`.
  - From `McpFeedback` (Explicit user command in chat): immediate promotion (`occurrences = 1`, `confidence = 0.95`).
  - From `AstScan` (Cold-start codebase ratio `>= 0.80` across `>= 10` files): immediate promotion (`confidence = ratio`).
- **Reinforcement Formula:**
  Each time the same pattern transformation is observed in `AstDiff`:
  $$\text{confidence}_{\text{new}} = \min\left(0.99, \text{confidence}_{\text{old}} + 0.20 \times (1.0 - \text{confidence}_{\text{old}})\right)$$
- **Promotion Threshold:**
  When `confidence >= 0.80` AND `occurrences >= 3` (or `source == McpFeedback`), the rule moves from SQLite `candidate_rules` into Git-tracked `.taste/rules.json` and triggers `taste sync`.
- **Contradiction Decay:**
  If a developer commits code matching the `anti_pattern` of an existing rule:
  $$\text{confidence}_{\text{new}} = \text{confidence}_{\text{old}} \times 0.75$$
  If `confidence < 0.50`, it is demoted from `.taste/rules.json` back to `candidate_rules`.

### 5.2 Deterministic AST Diff Analyzer (`diff/structural.rs`)

Instead of relying on cloud LLMs, `taste-core` identifies concrete structural transformations between `before_code` (AI generated) and `after_code` (Human edited):

1. **Decorator Swap Detector:** Detects when `@A(...)` on a class/method/property is replaced by `@B(...)`.
2. **Import Path Rewriter:** Detects when relative imports (`../..`) are systematically rewritten to path aliases (`@/`, `~/`) or vice-versa.
3. **Type Construct Swap:** Detects `interface X` $\leftrightarrow$ `type X =`.
4. **Error Class Substitution:** Detects `throw new Error(...)` or `HttpException` $\rightarrow$ custom exception class (e.g., `AppException`, `DomainError`).
5. **Control Flow Flattening:** Detects `if (...) { ... } else { ... }` refactored into early-return guard clauses (`if (!...) return;`).
6. **Function Syntax Preference:** Detects `function foo()` $\leftrightarrow$ `const foo = () =>`.
7. **Logger Replacement:** Detects `console.log/error` $\rightarrow$ custom logger service (`this.logger.log`, `logger.info`).

### 5.3 Secret Scrubber (`diff/scrubber.rs`)

Before saving any `anti_pattern` or `preferred_pattern` snippet:

- Redacts high-entropy strings (Shannon entropy $> 4.5$ on tokens $> 16$ chars) with `"<REDACTED_SECRET>"`.
  A flat 4.5 cannot catch hex (at most 4.0 bits per character) or tokens shorter than about 23 characters, so long hex tokens use a limit of 3.0 and mixed-case-plus-digit tokens of 20+ characters are measured against 90% of their length's maximum. Vendor key prefixes, webhook URLs, URL credentials and secret-named assignments are matched by pattern first.
- Strips `.env` values, Bearer tokens, AWS keys, and private keys.

---

## 6. Universal Transpiler (`crates/taste-core/src/transpiler/`)

### 6.1 Cursor Scoped Rules (`.cursor/rules/opentaste-<category>.mdc`)

Groups active rules (`enabled == true && confidence >= 0.80`) by category and scope, outputting valid Cursor `.mdc` frontmatter:

```markdown
---
description: OpenTaste auto-learned rules for Architecture (Do not edit manually)
globs: ["src/**/*.entity.ts", "src/**/*.service.ts"]
alwaysApply: false
---

# OpenTaste: Architecture Preferences

- **[src/**/\*.entity.ts]\*\* Use custom `@AutoEntity()` and `@AutoIndex()` decorators instead of raw TypeORM `@Entity()` and `@Index()`.
  - Avoid: `@Entity('users')`
  - Prefer: `@AutoEntity()`
```

### 6.2 Marker-Block Injection (`CLAUDE.md`, `.windsurfrules`, `AGENTS.md`)

Reads existing file (if present), preserves all manual developer content outside the markers, and injects/replaces only the block between `<!-- OPENTASTE:START -->` and `<!-- OPENTASTE:END -->`:

```markdown
<!-- OPENTASTE:START - AUTO-GENERATED BY OPENTASTE, DO NOT EDIT INSIDE THIS BLOCK -->

## Coding Taste & Project Conventions (Auto-Learned)

### Architecture

- **[`src/**/\*.entity.ts`]** Use custom `@AutoEntity()`and`@AutoIndex()`decorators instead of raw TypeORM`@Entity()`and`@Index()`.

### Code Style

- **[`**/\*`]\*\* Prefer early returns (guard clauses) over nested if-else blocks.
<!-- OPENTASTE:END -->
```

---

## 7. Local Stdio MCP Server (`crates/taste-mcp`)

Implements JSON-RPC 2.0 over `stdin`/`stdout` with zero external runtime dependencies.

### Exposed MCP Tools:

1. **`get_coding_taste`**
   - **Input:** `{ "file_path": "src/users/user.entity.ts" }`
   - **Output:** Returns all active rules whose `scope` glob matches `file_path`.
2. **`record_coding_taste`**
   - **Description:** _"Call this tool immediately whenever the user corrects your code style, architecture, naming, or explicitly states a coding preference."_
   - **Input:**
     ```json
     {
       "category": "architecture",
       "scope": "src/**/*.controller.ts",
       "instruction": "Always return standardized ApiResponse<T> wrapper from controller methods",
       "anti_pattern": "return this.userService.findAll();",
       "preferred_pattern": "return ApiResponse.ok(await this.userService.findAll());"
     }
     ```
   - **Action:** Saves rule with `confidence = 0.95`, updates `.taste/rules.json`, and runs `transpiler::sync_all()` immediately.
3. **`reject_coding_taste`**
   - **Input:** `{ "rule_id": "arch-custom-entity-decorators" }`
   - **Action:** Disables or deletes a rule that the user dislikes.

---

## 8. CLI & Interactive Terminal UI (`crates/taste-cli`)

### CLI Commands (`taste <subcommand>`)

- `taste init`:
  1. Creates `.taste/rules.json` and `.taste/cache.db`.
  2. Adds `.taste/cache.db` to `.gitignore`.
  3. Injects `.cursor/mcp.json` entry for `opentaste` (`{"command": "taste", "args": ["mcp"]}`).
  4. Installs `.git/hooks/post-commit` script (`taste learn --from-git-head &`).
  5. Runs initial `taste scan` and `taste sync`.
- `taste scan [--history N]`: Walks repository using `ignore::WalkParallel`, parses ASTs, populates initial high-confidence rules, optionally reads the last `N` commits of history, and runs `taste sync` when it confirmed or retracted anything.
- `taste sync`: Compiles `.taste/rules.json` into Cursor, Claude, Windsurf, and Copilot instruction files.
- `taste learn-delta --file <path>`: Reads JSON payload `{"before": "...", "after": "..."}` from `stdin`, runs structural diff analysis, updates candidate/confirmed rules, and prints any newly promoted rule as JSON.
- `taste status`: Prints a clean summary table of learned rules and synced editor targets.
- `taste ui`: Opens the `ratatui` interactive terminal dashboard.

### Terminal UI (`ratatui`) Specification:

- **Left Pane (45% width):** Filterable list of Confirmed Rules (`[✓]`) and Candidate Rules (`[?]`), displaying confidence percentage bar (`████░ 88%`) and category badge.
- **Right Pane (55% width):** Full details of selected rule: Scope glob, Occurrences count, Instruction text, and syntax-highlighted `Anti-Pattern (Avoid)` vs `Preferred-Pattern (Use)` diff box.
- **Bottom Status Bar (Keybindings):**
  `[↑/↓] Navigate` | `[Space] Toggle Enable` | `[p] Promote Candidate` | `[d] Delete` | `[s] Sync to Editors` | `[q] Quit`

---

## 9. Universal IDE Extension (`packages/vscode-opentaste`)

Works natively in **VS Code, Cursor, and Windsurf** (published to both VS Code Marketplace and Open VSX Registry).

### How `deltaTracker.ts` Captures "Post-AI Human Edits":

1. **Detecting an AI / Multi-line Generation Burst:**
   Listens to `vscode.workspace.onDidChangeTextDocument`. When a change event inserts `>= 4` lines of code in a single transaction (characteristic of Cursor Agent/Tab accept, Copilot accept, or paste), records an `AiBurstWindow`:
   - `filePath`, `startLine`, `endLine`, `aiSnapshotText`, `timestamp`.
2. **Capturing the Human Correction:**
   If the developer makes any subsequent edit within the same `startLine..endLine` range that is not itself a burst (typing, deletions, line modifications, a rename or a code action), within **180 seconds**, and then **saves the file (`onDidSaveTextDocument`, the only event that sees the saved text)**:
   - There is deliberately no size limit on that edit. The corrections the diff detectors learn from are often a single edit longer than 15 characters: pasting `'@/shared/errors'` over a relative import (17), `this.logger.log(` over `console.log(` (16), or a "convert to arrow function" code action (41 on one line). Precision comes from the detectors, which ignore anything that is not one of their exact transformations.
   - Extracts the `before` block (`aiSnapshotText`) and `after` block (current saved text in that range).
   - Spawns `taste learn-delta --file <relative_path>` via `stdin`.
3. **Non-Intrusive Feedback Toast:**
   If `taste learn-delta` returns a newly promoted rule, displays a VS Code information notification:
   `✨ OpenTaste learned: "Use @AutoEntity() instead of @Entity()" [Keep Rule] [Ignore]`

---

## 10. Step-by-Step Implementation Plan for Cursor Agent

Execute these phases sequentially. After completing each phase, run `cargo check` and `cargo test` to verify zero warnings/errors before moving to the next phase.

### Phase 1: Workspace Scaffolding, Models & Storage (`crates/taste-core`)

1. Initialize root `Cargo.toml` workspace and `crates/taste-core`, `crates/taste-mcp`, `crates/taste-cli`.
2. Implement `models.rs`, `config.rs`, `storage/json_store.rs` (`.taste/rules.json`), and `storage/sqlite_store.rs` (`.taste/cache.db`).
3. Implement `scorer/confidence.rs` and `scorer/dedup.rs` with unit tests verifying promotion at `confidence >= 0.80` and `occurrences >= 3`.

### Phase 2: Universal Transpiler (`crates/taste-core/src/transpiler/`)

1. Implement `marker.rs` for idempotent `<!-- OPENTASTE:START --> ... <!-- OPENTASTE:END -->` block injection.
2. Implement `cursor_mdc.rs`, `claude_md.rs`, `windsurf.rs`, and `agents_md.rs`.
3. Write integration tests verifying that existing user content in `CLAUDE.md` is never overwritten.

### Phase 3: AST Cold-Start Scanner & Structural Diff Engine (`crates/taste-core/src/ast/` & `diff/`)

1. Integrate `tree-sitter` and `tree-sitter-typescript` / `python` / `rust`.
2. Implement `ast/analyzers/` to detect import style (alias vs relative), type vs interface ratio, function style, custom decorators, and error classes across a repository in parallel using `ignore` + `rayon`.
3. Implement `diff/structural.rs` and `diff/scrubber.rs` to extract candidate rules from `(before_code, after_code)` pairs.

### Phase 4: MCP Server (`crates/taste-mcp`)

1. Build the JSON-RPC 2.0 `stdio` loop in `taste-mcp`.
2. Implement `tools/list` and `tools/call` for `get_coding_taste`, `record_coding_taste`, and `reject_coding_taste`.

### Phase 5: CLI & Ratatui TUI (`crates/taste-cli`)

1. Wire up `clap` commands: `taste init`, `taste scan`, `taste sync`, `taste learn-delta`, `taste status`, `taste mcp`, and `taste ui`.
2. Implement the interactive `ratatui` TUI in `crates/taste-cli/src/tui/`.

### Phase 6: VS Code / Cursor Extension & NPM Wrapper (`packages/`)

1. Create `packages/vscode-opentaste` with `deltaTracker.ts`, `binaryRunner.ts`, and `statusBar.ts`.
2. Create `packages/npm-opentaste` wrapper so `npx opentaste` invokes the compiled binary cleanly.
