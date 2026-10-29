import { strict as assert } from 'node:assert';
import { test } from 'node:test';

import { DeltaTracker, insertedLines } from '../deltaTracker';
import { isLearnable, type Targets } from '../targets';
import { FakeDocument } from './fakeDocument';

const TARGETS: Targets = {
  marker_targets: ['CLAUDE.md', '.windsurfrules', 'AGENTS.md', '.github/copilot-instructions.md'],
  cursor_rules_dir: '.cursor/rules',
  cursor_rule_prefix: 'opentaste-',
  cursor_rule_extension: 'mdc',
};

const FILE = 'src/users/user.entity.ts';
const AI_BLOCK = '@Entity("users")\nexport class User {\n  id: string;\n}\n';

function setup() {
  let clock = 1_000_000;
  const tracker = new DeltaTracker({
    now: () => clock,
    isLearnable: (file) => isLearnable(file, TARGETS),
  });
  const doc = new FakeDocument('// header\n\nexport {};\n');
  const advance = (ms: number) => {
    clock += ms;
  };
  return { tracker, doc, advance };
}

test('inserted line counting treats a trailing newline as the end of the last line', () => {
  assert.equal(insertedLines(''), 0);
  assert.equal(insertedLines('x'), 1);
  assert.equal(insertedLines('a\nb\nc\nd'), 4);
  assert.equal(insertedLines('a\nb\nc\nd\n'), 4);
  assert.equal(insertedLines('a\r\nb\r\nc\r\nd\r\n'), 4);
});

test('a genuine four-line burst opens a window over exactly the inserted lines', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  const [window] = tracker.windowsFor(FILE);
  assert.ok(window);
  assert.equal(window.startLine, 1);
  assert.equal(window.endLine, 4);
  assert.equal(window.aiSnapshotText, AI_BLOCK.trimEnd());
  assert.equal(window.corrected, false);
});

test('three lines are not a burst', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, 'a\nb\nc\n')], undefined, doc.read);
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('an undo or redo never opens a window and closes the ones that were open', () => {
  for (const reason of ['undo', 'redo'] as const) {
    const { tracker, doc } = setup();
    tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
    assert.equal(tracker.windowsFor(FILE).length, 1);
    tracker.onChange(FILE, [doc.insertAt(0, 'x\ny\nz\nw\nv\n')], reason, doc.read);
    assert.equal(tracker.windowsFor(FILE).length, 0, reason);
  }
});

test('an edit above an open window shifts it, so the after text is the same block', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);

  // Two imports added above the block, then one line removed above it.
  tracker.onChange(FILE, [doc.insertAt(0, 'import a from "a";\nimport b from "b";\n')], undefined, doc.read);
  assert.deepEqual([tracker.windowsFor(FILE)[0]?.startLine, tracker.windowsFor(FILE)[0]?.endLine], [3, 6]);
  tracker.onChange(FILE, [doc.deleteLines(0, 0)], undefined, doc.read);
  assert.deepEqual([tracker.windowsFor(FILE)[0]?.startLine, tracker.windowsFor(FILE)[0]?.endLine], [2, 5]);

  // The human corrects the decorator line inside the block, and saves.
  tracker.onChange(FILE, [doc.editLine(2, '@AutoEntity()')], undefined, doc.read);
  const deltas = tracker.onSave(FILE, doc.read);
  assert.equal(deltas.length, 1);
  assert.equal(deltas[0]?.before, AI_BLOCK.trimEnd());
  assert.equal(deltas[0]?.after, '@AutoEntity()\nexport class User {\n  id: string;\n}');
});

test('a line inserted right at the top of a window moves it without touching it', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.insertAt(1, 'import { AutoEntity } from "@/db";\n')], undefined, doc.read);
  const [window] = tracker.windowsFor(FILE);
  assert.deepEqual([window?.startLine, window?.endLine, window?.corrected], [2, 5, false]);
  assert.equal(doc.read(2, 5), AI_BLOCK.trimEnd(), 'the window still covers the AI block');

  // Text typed at the start of the first line, without a newline, is an edit.
  tracker.onChange(FILE, [doc.insertAt(2, '// ')], undefined, doc.read);
  assert.equal(tracker.windowsFor(FILE)[0]?.corrected, true);
});

test('edits below a window leave it where it is and uncorrected', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.insertAt(7, 'const later = 1;\n')], undefined, doc.read);
  const [window] = tracker.windowsFor(FILE);
  assert.deepEqual([window?.startLine, window?.endLine, window?.corrected], [1, 4, false]);
  assert.deepEqual(tracker.onSave(FILE, doc.read), [], 'an untouched block teaches nothing');
  assert.equal(tracker.windowsFor(FILE).length, 1, 'and stays open for a later correction');
});

test('a line added inside the window grows it, so the after text includes it', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.insertAt(4, '  name: string;\n')], undefined, doc.read);
  const deltas = tracker.onSave(FILE, doc.read);
  assert.equal(deltas[0]?.after, '@Entity("users")\nexport class User {\n  id: string;\n  name: string;\n}');
});

test('writes to the transpiler targets and other non-source files are ignored', () => {
  const { tracker, doc } = setup();
  for (const file of [
    'CLAUDE.md',
    'AGENTS.md',
    '.windsurfrules',
    '.github/copilot-instructions.md',
    '.cursor/rules/opentaste-imports.mdc',
    'README.md',
    'node_modules/pkg/index.ts',
    'src/types.d.ts',
  ]) {
    tracker.onChange(file, [doc.insertAt(0, 'a\nb\nc\nd\ne\n')], undefined, doc.read);
    assert.equal(tracker.windowsFor(file).length, 0, file);
  }
  assert.ok(isLearnable('src/app.tsx', TARGETS));
  assert.ok(isLearnable('.cursor/rules/../../src/app.ts'.replace('.cursor/rules/../../', ''), TARGETS));
  assert.ok(!isLearnable('../elsewhere/app.ts', TARGETS));
});

test('a window older than 180 seconds does not fire', () => {
  const { tracker, doc, advance } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.editLine(1, '@AutoEntity()')], undefined, doc.read);
  advance(180_001);
  assert.deepEqual(tracker.onSave(FILE, doc.read), []);
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('a window inside its lifetime fires once and then closes', () => {
  const { tracker, doc, advance } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.editLine(1, '@AutoEntity()')], undefined, doc.read);
  advance(179_000);
  assert.equal(tracker.onSave(FILE, doc.read).length, 1);
  assert.deepEqual(tracker.onSave(FILE, doc.read), [], 'a second save does not resend it');
});

test('a correction longer than fifteen characters, like a code action, still counts', () => {
  const { tracker, doc } = setup();
  const block = 'export function findUser(id: string) {\n  return db.get(id);\n}\nexport const x = 1;\n';
  tracker.onChange(FILE, [doc.insertAt(1, block)], undefined, doc.read);
  // "Convert to arrow function" rewrites the header line in one edit.
  tracker.onChange(FILE, [doc.editLine(1, 'export const findUser = (id: string) => {')], undefined, doc.read);
  const deltas = tracker.onSave(FILE, doc.read);
  assert.equal(deltas.length, 1);
  assert.equal(deltas[0]?.before.split('\n')[0], 'export function findUser(id: string) {');
  assert.equal(deltas[0]?.after.split('\n')[0], 'export const findUser = (id: string) => {');
});

test('expiry drops stale windows without a save', () => {
  const { tracker, doc, advance } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  advance(60_000);
  tracker.expire();
  assert.equal(tracker.windowsFor(FILE).length, 1);
  advance(121_000);
  tracker.expire();
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('the number of live windows per document is capped, oldest first out', () => {
  const { tracker, doc } = setup();
  for (let i = 0; i < 12; i++) {
    tracker.onChange(FILE, [doc.insertAt(0, `// burst ${i}\na\nb\nc\n`)], undefined, doc.read);
  }
  const windows = tracker.windowsFor(FILE);
  assert.equal(windows.length, 8);
  assert.equal(windows[windows.length - 1]?.aiSnapshotText.split('\n')[0], '// burst 11');
});

test('a multi-cursor transaction does not open a window', () => {
  const { tracker, doc } = setup();
  const first = doc.insertAt(2, 'a\nb\nc\nd\n');
  const second = doc.insertAt(0, 'e\nf\ng\nh\n');
  tracker.onChange(FILE, [first, second], undefined, doc.read);
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('a new burst over an open window replaces it', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.insertAt(2, 'one\ntwo\nthree\nfour\n')], undefined, doc.read);
  const windows = tracker.windowsFor(FILE);
  assert.equal(windows.length, 1);
  assert.equal(windows[0]?.aiSnapshotText, 'one\ntwo\nthree\nfour');
});

test('deleting the whole block closes its window', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.deleteLines(1, 4)], undefined, doc.read);
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('an edit that starts above the window and runs into it makes it ambiguous, so it closes', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.replace({ line: 0, character: 3 }, { line: 2, character: 5 }, 'x')], undefined, doc.read);
  assert.equal(tracker.windowsFor(FILE).length, 0);
});

test('a correction that restores the AI text exactly teaches nothing', () => {
  const { tracker, doc } = setup();
  tracker.onChange(FILE, [doc.insertAt(1, AI_BLOCK)], undefined, doc.read);
  tracker.onChange(FILE, [doc.editLine(1, '@Entity("people")')], undefined, doc.read);
  tracker.onChange(FILE, [doc.editLine(1, '@Entity("users")')], undefined, doc.read);
  assert.deepEqual(tracker.onSave(FILE, doc.read), []);
});
