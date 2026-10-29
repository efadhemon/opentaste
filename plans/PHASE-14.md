# Phase 14: Release v1.0.0

**Status:** pending

| | |
| --- | --- |
| **Blocked by** | Every phase from 7 to 13 `completed`, including their manual gates |
| **Blocks** | Nothing. This is the release. |
| **Rough size** | 1 session, plus the user's account steps |

Phase 6's open gates (the editor check and the clean-machine `npx` check)
move here: they can only be done against a published release.

---

## Step 1: Account and repository setup (the user's)

- [ ] npm: the `@opentaste` organisation, and `NPM_TOKEN` (granular,
      read and write, scoped to `@opentaste`, bypass 2FA) in the `release`
      environment. *Done on 2026-10-30.*
- [ ] VS Code Marketplace: the `opentaste` publisher, and `VSCE_PAT` in the
      `release` environment.
- [ ] Open VSX: the `opentaste` namespace, the publisher agreement, and
      `OVSX_PAT` in the `release` environment.
- [ ] GitHub Pages enabled for the website, served from Actions.

## Step 2: Guard the release workflow

- [ ] `release.yml` refuses to publish a tag whose commit is not on `main`
      (AGENTS.md already says tags are only cut on `main`; this makes it
      true). Test it with a dry run from a branch.

## Step 3: Version 1.0.0

- [ ] Bump the version in the workspace `Cargo.toml`,
      `packages/npm-opentaste/package.json` and
      `packages/vscode-opentaste/package.json` together; the release
      workflow already fails if they disagree with the tag.
- [ ] `CHANGELOG.md`, starting at 1.0.0, written for users: what OpenTaste
      does, every supported editor and language, and the known limits.
- [ ] Dry-run the release workflow (`workflow_dispatch`) on `dev`: all
      targets build, the npm tarballs and the VSIX are produced.

## Step 4: Release

- [ ] A `dev` → `main` pull request with the full CI green; merged by the
      user.
- [ ] Tag `v1.0.0` on `main` and push it; the workflow publishes the npm
      packages, the Marketplace and Open VSX extension, and the GitHub
      release with binaries.
- [ ] The website launches (WEBSITE.md step 7) and the 1.0.0 announcement
      post is published.

## Step 5: Verify the published release

- [ ] `npx @opentaste/cli status` on a machine with no Rust toolchain, on
      Linux, macOS and Windows.
- [ ] `npm install --global @opentaste/cli` then `taste init` in a real
      project, and the post-commit hook firing.
- [ ] **Manual gate (from phase 6):** install the extension from the
      Marketplace in VS Code and from Open VSX in Cursor and Windsurf;
      confirm in each that a real AI edit followed by a correction produces
      a learned rule and the notification.
- [ ] Every install command on the website works as written.

## Step 6: After the release

- [ ] Move npm publishing to trusted publishing (OIDC): configure each of the
      six packages on npmjs.com for `efadhemon/opentaste`, `release.yml`,
      environment `release`; remove `NPM_TOKEN` from the workflow; revoke the
      token.
- [ ] Phase 6 and this file set to `completed`.

---

## Definition of done

- [ ] v1.0.0 is on npm, the VS Code Marketplace, Open VSX and GitHub
      releases.
- [ ] Every step-5 check passed and recorded here.
- [ ] Trusted publishing in place and the token revoked.
- [ ] Status at the top of this file set to `completed`.
