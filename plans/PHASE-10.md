# Phase 10: Learning from recent git history

**Status:** completed

| | |
| --- | --- |
| **Blocked by** | [Phase 7](PHASE-7.md) (change fingerprints so nothing counts twice), [Phase 8](PHASE-8.md) and [Phase 9](PHASE-9.md) (history should be read with every detector, once) |
| **Blocks** | Nothing |
| **Rough size** | 2 sessions |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) M1, W13 |

SPEC.md §1 says the cold-start scanner reads "existing source files **and
recent Git history**". Today `taste init` sees only the working tree, and the
post-commit hook only sees commits made after it was installed. A project
with years of corrections starts from nothing.

AGENTS.md deferred this because "walking back through history multiplies the
cost of a wrong detector across every commit ever made." The plan bounds that
cost: a window, not all of history; corrections only, not decay; and every
change counted once.

---

## Step 1: What counts as recent

- [x] `taste scan --history <N>` reads the last `N` non-merge commits on the
      current branch (first parent only). `taste init` runs it with a default
      of **200 commits or 180 days, whichever is fewer**. `--history 0`
      turns it off. (`taste-cli/src/commands/history.rs`. Plain `taste scan`
      reads no history; `--history N` has no age limit.)
- [x] Skipped commits: merges; commits that touch more than 50 files
      (formatting sweeps, vendoring, generated code are not taste); commits
      whose author is a known bot (`dependabot`, `renovate`,
      `github-actions`); commits whose message marks them as a revert.
      (Also any author or email with `[bot]` in it.)
- [x] Tests: the window stops at whichever limit comes first; each skip rule
      with a fixture repository built by the test. (`taste-cli/tests/history.rs`.)

## Step 2: Learning from a past commit

- [x] Each remaining commit is diffed against its first parent and passed,
      file by file, through `analyze_in`, exactly as the post-commit hook
      does, oldest first.
- [x] Every change is fingerprinted with the `learned_changes` table from
      PR #10, so mining twice, or mining and then the hook seeing the same
      commit, counts once. (`change_fingerprint` moved to `commands/mod.rs`
      so both use the same one; editor deltas are honoured as the hook does.)
- [x] History records **corrections only**. It never applies contradictions:
      code written before a rule existed is not the developer working
      against it.
- [x] A cursor in the cache remembers the newest commit mined, so a later
      `taste scan --history` only reads commits after it. (New `cache_meta`
      table, schema version 5. A cursor that is no longer an ancestor after a
      rebase is ignored; fingerprints still prevent double counting.)
- [x] Tests: a correction made in five past commits reaches the candidate at
      the count five sightings give; mining twice changes nothing; mining
      then running the hook on `HEAD` counts once; a past commit adding an
      anti-pattern does not decay a rule.
- [x] *Added by the manual gate:* **a past correction the code has since
      reversed is left out.** create-t3-turbo's history held arrow-function
      corrections in `packages/ui`, which today is 31 `function` declarations
      to 9 arrows; read oldest first they outran the real style. For a
      symmetric choice, a history observation whose other side is the
      majority in the working tree's tally is dropped (`current_sides`).
- [x] *Added by the manual gate:* **a correction towards one side decays a
      candidate for the other side in the same scope**, as a contradiction
      would (`RuleStore::decay_opposite_candidates`). This applies to every
      learner, not only history: without it a team that switched styles
      keeps both candidates until one wins by count.

## Step 3: Speed

- [x] Blobs are read with one `git cat-file --batch` process rather than a
      `git show` per file. (`git::Blobs`.)
- [x] Analysis runs in parallel across files with `rayon`; recording stays
      serial, through `RuleStore`.
- [x] `taste init` prints progress (`Reading history: 120/200 commits`) and
      can be interrupted without leaving the cache inconsistent. (A file's
      fingerprint is stored before its observations and the cursor after
      each commit: a stopped run resumes and at worst misses one file's
      correction, never counts one twice. Tested by rewinding the cursor.)
- [x] Target: 200 commits of a mid-sized repository in under 5 seconds in a
      release build, measured and recorded in this file.

      | Repository (full clone) | Commits read / skipped | Wall time |
      | --- | --- | --- |
      | t3-oss/create-t3-turbo (946 commits) | 150 / 50 (48 bots, 2 reverts) | 0.55 s |
      | pallets/flask (1,252 commits) | 190 / 10 (8 bots, 1 revert, 1 sweep) | 0.89 s |
      | fastapi/full-stack-fastapi-template (1,539 commits) | 65 / 135 (bots) | 0.44 s |

      Release build, `taste scan --history 200` on 2026-10-30.

## Step 4: Reporting, and syncing after a scan

- [x] `taste scan` runs `taste sync` when it confirmed or demoted anything,
      like every other learner; SPEC.md §5.1 says promotion triggers a sync,
      and today only `init` and `watch` do it after a scan (V1-AUDIT W13).
      Test: a standalone scan that confirms a rule leaves the editor files
      up to date. (Also when a compiler-setting rule was retracted.)
- [x] `taste status` shows how much history was read and when.
- [x] Rules first seen in history are recorded with source `ast-diff`, the
      same as any correction; the scan report says how many came from
      history.

## Step 5: The full test set

- [x] Window limits and every skip rule.
- [x] Oldest-first order, fingerprints, the cursor.
- [x] No contradictions from history.
- [x] Interrupting a run leaves the cache valid (the next run resumes).

## Manual gate record (2026-10-30)

`taste init` and `taste scan --history 200` on the three full clones above.
History produced **no confirmed rule** and one candidate:

| Repository | Candidate | Sightings | Verdict | Evidence |
| --- | --- | --- | --- | --- |
| create-t3-turbo | `packages/ui/**/*.tsx`: prefer `function` declarations | 3 (65%) | yes | HEAD `packages/ui` is 31 declarations to 9 arrows; `shadcn React 19` (#1243), `bump expo to v54` (#1413) and `migrate to <Field />` (#1433) converted components to declarations |

Before the gate's two fixes the same run left "prefer arrow functions" ahead
(56% against 37%), the opposite of the code. flask and the FastAPI template
gave no corrections from history: their human commits contain none of the
swaps the detectors know.

The maintainer answered yes on 2026-10-31.

---

## Definition of done

- [x] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean (463 tests).
- [x] **Manual gate:** run `taste init` on two real repositories with long
      histories, and agree with every rule that history produced. Record
      them here.
- [x] The speed target met and recorded.
- [x] AGENTS.md's history row removed; SPEC.md §1 describes the window.
- [x] V1-AUDIT M1 and W13 marked fixed.
- [x] Status at the top of this file set to `completed`.
