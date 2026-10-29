# OpenTaste for VS Code, Cursor and Windsurf

OpenTaste learns your project's coding taste from the corrections you make to
AI-written code, and writes it into the instruction files your AI tools read:
`CLAUDE.md`, `AGENTS.md`, `.windsurfrules`, `.github/copilot-instructions.md`
and `.cursor/rules/`.

When an AI tool inserts a block of four or more lines and you correct it within
three minutes, saving the file hands the before and after to the local `taste`
binary. When the same correction has been seen often enough, you get one quiet
notification:

> ✨ OpenTaste learned: "Use `@AutoEntity` instead of `@Entity`." [Keep Rule] [Ignore]

**Ignore** disables the rule; nothing is deleted. Undo and redo are never
learned from, and neither are OpenTaste's own instruction files.

## Requirements

- The `taste` binary, from `npm install -D @opentaste/cli` in the project, or
  on your `PATH`, or set with `opentaste.binaryPath`.
- A project set up with `taste init`. The extension does nothing anywhere else.

Everything runs locally. There is no network access and no telemetry.

## Settings

- `opentaste.binaryPath`: where `taste` is, if not in `node_modules/.bin` or on `PATH`.
- `opentaste.learnFromEdits`: turn learning from edits off without uninstalling.

## Commands

- **OpenTaste: Open Dashboard** runs `taste ui` in a terminal.
- **OpenTaste: Sync Rules to Editors** runs `taste sync`.
