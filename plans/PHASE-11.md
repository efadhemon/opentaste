# Phase 11: Cross-project rules (`~/.taste/global.db`)

**Status:** completed

| | |
| --- | --- |
| **Blocked by** | [Phase 7](PHASE-7.md). Independent of phases 8–10. |
| **Blocks** | Nothing |
| **Rough size** | 2–3 sessions, after the design in step 1 is approved |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) M6 |

SPEC.md names `~/.taste/global.db` three times (§2 `config.rs`, §2
`sqlite_store.rs`, §4.3) and says it uses the same schema as the project
cache, but never says what goes in it or who sees it. `Paths::global_db()`
exists; nothing reads or writes the file.

AGENTS.md deferred it because "sharing taste across unrelated repositories is
a separate feature with its own privacy questions." Those questions are
answered in step 1, in writing, before any code.

---

## Step 1: Design, approved before code

The proposal below is the starting point. The step is done when the user has
read it and approved or changed it, and this section records the decision.

**Approved as written by the maintainer on 2026-10-31.**

- **What is global.** Two things, both on the developer's own machine:
  1. *Personal rules* the developer explicitly marks as theirs
     (`taste global add <rule-id>` in a project, or `record_coding_taste`
     with `"global": true`). Never automatic.
  2. *Suggestions:* a rule confirmed in two or more unrelated projects is
     offered as a personal rule (`taste global suggest`). It becomes one only
     when accepted.
- **What is stored.** The same rule fields as `rules.json`, plus the projects
  it was confirmed in, as paths. Snippets are kept only if the developer
  says so when adding the rule, since they are code from a specific project.
- **Where global rules reach the editor.** Never in a project's committed
  files: a personal preference must not land in a team's `CLAUDE.md`. They
  are delivered through:
  - `get_coding_taste` (MCP), in every project, marked as personal;
  - user-level instruction files for editors that have them (for example
    `~/.claude/CLAUDE.md`, or a global rules file where the editor supports
    one), inside OpenTaste's marker block, with the same non-destructive
    splicing as project files.
- **Precedence.** A project rule wins. A personal rule whose opposite is
  confirmed in the current project is not served there.
- **Opting out.** `taste global off` in a project stops personal rules being
  served there (stored in the project's local cache, not in git).
- **Nothing leaves the machine.** No sync service, no account.

## Step 2: Storage

- [x] `global.db` with the cache schema plus a `personal_rules` table and a
      `seen_in` table (rule id, project path). Opened lazily; a missing home
      directory is a clear error, never a crash. (`storage/global.rs`. The
      cache's own tables are left out: candidates, snapshots and learned
      changes are always about one project, so global.db holds only its two
      tables. The home directory is `$TASTE_HOME` when set, so tests never
      touch the real one, and `Error::HomeDirNotFound` otherwise.)
- [x] `RuleStore` gains read access to personal rules and keeps the project
      lock rules: global writes take their own lock in `~/.taste/`.
      (`RuleStore::with_global`, `personal_rules_for_file`,
      `set_global_off`. Every promotion records a sighting; a home that
      cannot be written never stops a project learning.)
- [x] Tests: schema creation and migration; concurrent writes from two
      projects serialise. (Also suggestions, snippets, precedence and
      opting out, in `storage/global.rs` and `tests/global.rs`.)

## Step 3: Commands and MCP

- [x] `taste global add <id>`, `taste global list`, `taste global remove
      <id>`, `taste global suggest`, `taste global off|on`. (`add` takes a
      confirmed rule here or a suggested one, with `--keep-snippets` to take
      its code along.)
- [x] `get_coding_taste` returns matching personal rules after project
      rules, each marked as personal; `record_coding_taste` accepts
      `"global": true`. (Without snippets, which are the project's code.)
- [x] Every confirmation in a project records the project in `seen_in`, so
      suggestions can be computed. (Including the scan inside `taste init`,
      which opened its store without personal rules attached until the CLI
      test for suggestions failed on it.)
- [x] Tests: each command; precedence over an opposite project rule;
      opting out; MCP output marks personal rules. Every test that runs the
      binary sets `TASTE_HOME` to a temporary directory, the extension's
      end-to-end test included, so the suite never writes to the real
      `~/.taste`.

## Step 4: User-level instruction files

- [x] `taste sync` also writes personal rules into the user-level files of
      the editors that read one, using `transpiler/marker.rs`. Which files
      those are is one list, owned by `config::Paths`, like the project
      targets. (`Paths::user_targets`: `~/.claude/CLAUDE.md`,
      `~/.codeium/windsurf/memories/global_rules.md`, `~/.codex/AGENTS.md`,
      each only when that editor's directory exists. Cursor and Copilot keep
      user rules in their settings, not a file, so they get personal rules
      through MCP only. The user-level file lists every enabled personal
      rule; per-project precedence applies through MCP, and the block says a
      project's own conventions come first.)
- [x] Never writes personal rules into any project file.
- [x] Tests: user-level file created and spliced without touching existing
      content; project files never contain a personal rule. (Also: no folder
      created for an editor that is not installed, and the blocks withdrawn
      when the last personal rule goes.)

## Step 5: Dashboard and status

- [x] `taste status` lists personal rules served in this project. (Or says the project opted out.)
- [x] The dashboard shows them with a "personal" badge and can add or
      remove them. (`[g]` toggles. A rule this project has and is personal is
      one badged row; a personal rule from elsewhere is a `[★]` row of its own
      that cannot be deleted or promoted here.)

## Step 6: The full test set

- [x] Storage, locking and migration.
- [x] Every command, MCP field and precedence rule.
- [x] Personal rules never in project files; user-level files spliced safely.

---

## Definition of done

- [x] Step 1's design approved and recorded.
- [x] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean (482 tests).
- [x] **Manual gate:** two real projects on one machine; a rule added as
      personal in one is served in the other through MCP and the user-level
      file, and never appears in either project's git diff.
      Run on 2026-10-31 with a release build and a scratch `TASTE_HOME`
      holding a `.claude` directory with the user's own `CLAUDE.md`, on clones
      of create-t3-turbo and full-stack-fastapi-template. In create-t3-turbo,
      `taste global add style-early-return-guard-clauses-nested-19d26d`. In
      the FastAPI template, `taste status` listed it under "Personal rules
      served here (1)", and `get_coding_taste` for `frontend/src/main.tsx`
      returned it as `(personal)` after the project's rules. The user-level
      `CLAUDE.md` gained the "Personal Coding Preferences" block below the
      user's notes, untouched. Neither project's `CLAUDE.md`, `AGENTS.md`,
      `.windsurfrules` or Copilot file mentions it, and each `git diff` is
      only `.gitignore`'s OpenTaste cache entries. The real `~/.taste` was
      never created.
- [x] The website gains a page on personal rules (phase 13 checks this).
      *Handed to phase 13 step 5*, which already lists a page for personal
      rules: the site lives on the unmerged `feat/website` branch. SPEC.md
      §4.3 is the source to write it from.
- [x] AGENTS.md's global row removed; SPEC.md §4.3 says what `global.db`
      holds.
- [x] V1-AUDIT M6 marked fixed.
- [x] Status at the top of this file set to `completed`.
