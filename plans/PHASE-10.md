# Phase 10: Learning from recent git history

**Status:** pending

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

- [ ] `taste scan --history <N>` reads the last `N` non-merge commits on the
      current branch (first parent only). `taste init` runs it with a default
      of **200 commits or 180 days, whichever is fewer**. `--history 0`
      turns it off.
- [ ] Skipped commits: merges; commits that touch more than 50 files
      (formatting sweeps, vendoring, generated code are not taste); commits
      whose author is a known bot (`dependabot`, `renovate`,
      `github-actions`); commits whose message marks them as a revert.
- [ ] Tests: the window stops at whichever limit comes first; each skip rule
      with a fixture repository built by the test.

## Step 2: Learning from a past commit

- [ ] Each remaining commit is diffed against its first parent and passed,
      file by file, through `analyze_in`, exactly as the post-commit hook
      does, oldest first.
- [ ] Every change is fingerprinted with the `learned_changes` table from
      PR #10, so mining twice, or mining and then the hook seeing the same
      commit, counts once.
- [ ] History records **corrections only**. It never applies contradictions:
      code written before a rule existed is not the developer working
      against it.
- [ ] A cursor in the cache remembers the newest commit mined, so a later
      `taste scan --history` only reads commits after it.
- [ ] Tests: a correction made in five past commits reaches the candidate at
      the count five sightings give; mining twice changes nothing; mining
      then running the hook on `HEAD` counts once; a past commit adding an
      anti-pattern does not decay a rule.

## Step 3: Speed

- [ ] Blobs are read with one `git cat-file --batch` process rather than a
      `git show` per file.
- [ ] Analysis runs in parallel across files with `rayon`; recording stays
      serial, through `RuleStore`.
- [ ] `taste init` prints progress (`Reading history: 120/200 commits`) and
      can be interrupted without leaving the cache inconsistent.
- [ ] Target: 200 commits of a mid-sized repository in under 5 seconds in a
      release build, measured and recorded in this file.

## Step 4: Reporting, and syncing after a scan

- [ ] `taste scan` runs `taste sync` when it confirmed or demoted anything,
      like every other learner; SPEC.md §5.1 says promotion triggers a sync,
      and today only `init` and `watch` do it after a scan (V1-AUDIT W13).
      Test: a standalone scan that confirms a rule leaves the editor files
      up to date.
- [ ] `taste status` shows how much history was read and when.
- [ ] Rules first seen in history are recorded with source `ast-diff`, the
      same as any correction; the scan report says how many came from
      history.

## Step 5: The full test set

- [ ] Window limits and every skip rule.
- [ ] Oldest-first order, fingerprints, the cursor.
- [ ] No contradictions from history.
- [ ] Interrupting a run leaves the cache valid (the next run resumes).

---

## Definition of done

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
      warnings` and `cargo test --workspace` clean.
- [ ] **Manual gate:** run `taste init` on two real repositories with long
      histories, and agree with every rule that history produced. Record
      them here.
- [ ] The speed target met and recorded.
- [ ] AGENTS.md's history row removed; SPEC.md §1 describes the window.
- [ ] V1-AUDIT M1 and W13 marked fixed.
- [ ] Status at the top of this file set to `completed`.
