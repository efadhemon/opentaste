# Phase 13: Documentation and website

**Status:** pending

| | |
| --- | --- |
| **Blocked by** | Phases 8–12, so the docs describe what ships. The website's steps 1–3 are already done on `feat/website`. |
| **Blocks** | [Phase 14](PHASE-14.md) |
| **Rough size** | 2–3 sessions |
| **Rows** | [V1-AUDIT.md](V1-AUDIT.md) D1–D4, D6, M11; [WEBSITE.md](WEBSITE.md) steps 4–7 |

A wrong doc misleads the same way a wrong rule does. Every claim in this phase
is checked against the code, the way the website's step 3 was.

---

## Step 1: README (D1)

- [ ] Rewrite `README.md`: what OpenTaste does, install, a 30-second example,
      supported editors and languages, links to the website and docs. The
      "under construction / not yet implemented" text goes.
- [ ] Development section kept, with the pnpm commands next to the cargo
      ones.

## Step 2: SPEC.md matches the code (D2, D3, D4)

SPEC.md stays the architecture specification, so it must describe the
architecture that exists.

- [ ] §2 and §10: `@opentaste/cli` instead of `npx opentaste`, with the
      reason (M11, below); `reject_coding_taste` everywhere (§2 says
      `reject_taste`).
- [ ] §3: the workspace's real dependency versions and the 1.90 MSRV, or a
      pointer to `Cargo.toml` instead of a copy that drifts.
- [ ] §5.1: demotion as phase 7 made it reachable; the 0.80 editor-file
      threshold stated explicitly.
- [ ] Every section phases 8–12 changed: languages and dimensions (§2, §5.2),
      the history window (§1), `global.db` contents (§4.3), dashboard edit
      and highlighting (§8), measured scan speed (§1).
- [ ] A test-free but mechanical check: every SPEC.md section is listed in
      [V1-TRACEABILITY.md](V1-TRACEABILITY.md) against the code that
      implements it, with no row left open.

## Step 3: AGENTS.md (D6)

- [ ] The repository layout, the baseline test counts, and the phase table
      with phases 7–14.
- [ ] The "not deferred any more" table is removed once its last row has
      shipped.

## Step 4: The npm name (M11)

- [ ] Decide: keep `@opentaste/cli` (recommended), and optionally ask npm
      support for the unscoped `opentaste` name under their dispute policy.
      Record the decision in SPEC.md and here. No code change unless the
      name is obtained.

## Step 5: The website

- [ ] Rebase `feat/website` onto `dev` and update its pages for phase 7:
      contradictions now demote below 0.50; the double-count and MCP-snippet
      warnings are removed (both fixed); monorepo scopes; the correction
      heuristic.
- [ ] [WEBSITE.md](WEBSITE.md) step 4: reference pages generated from the
      code and checked by tests (CLI, sync targets, MCP tools, `rules.json`,
      extension settings, detectors).
- [ ] WEBSITE.md step 5: concepts and architecture pages with diagrams.
- [ ] WEBSITE.md step 6: blog posts, about, roadmap; the user reads every
      post.
- [ ] Pages for what phases 8–12 add: Python, Rust and Go; the new
      TypeScript detectors; history mining; personal rules; dashboard
      editing. For the three languages, document every dimension, detector
      and exclusion recorded in [PHASE-8.md](PHASE-8.md) steps 2–4, and the
      256 KiB and versioned-file-name limits the scanner gained there.
- [ ] WEBSITE.md step 7 up to, not including, the launch itself: link
      checker in CI and the Pages workflow. Launch is part of phase 14.

---

## Definition of done

- [ ] README, SPEC.md and AGENTS.md describe the code on `dev`, checked line
      by line against it.
- [ ] V1-TRACEABILITY.md has no open row.
- [ ] The website builds with no broken links and every reference page is
      generated and tested.
- [ ] **Manual gate:** the user reads the README, the getting-started pages
      and every blog post.
- [ ] V1-AUDIT D1–D4, D6 and M11 marked fixed.
- [ ] Status at the top of this file set to `completed`.
