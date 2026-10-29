// The whole learning path against a real `taste` binary: an AI burst, a
// human correction, a save, `learn-delta`, until a rule is promoted.
//
// Runs only when TASTE_BIN points at a built binary, for example:
//   cargo build -p taste-cli && TASTE_BIN=$PWD/target/debug/taste pnpm -r run test

import { strict as assert } from 'node:assert';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { test } from 'node:test';

import { learnDelta, run } from '../binaryRunner';
import { DeltaTracker } from '../deltaTracker';
import { isLearnable, parseTargets } from '../targets';
import { FakeDocument } from './fakeDocument';

const binary = process.env.TASTE_BIN;

test('a corrected AI edit becomes a rule through the real binary', { skip: !binary && 'set TASTE_BIN' }, async () => {
  const taste = binary as string;
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'opentaste-e2e-')));
  assert.equal((await run(taste, ['init', '--quiet'], { cwd: root })).code, 0);
  const targets = parseTargets((await run(taste, ['targets', '--json'], { cwd: root })).stdout);
  assert.ok(targets, 'taste targets --json answers in the shape the extension reads');

  const file = 'src/users/user.entity.ts';
  let promoted: string | undefined;
  for (let attempt = 0; attempt < 8 && !promoted; attempt++) {
    const tracker = new DeltaTracker({ isLearnable: (f) => isLearnable(f, targets) });
    const doc = new FakeDocument('');
    tracker.onChange(file, [doc.insertAt(0, '@Entity("users")\nexport class User {\n  id: string;\n}\n')], undefined, doc.read);
    tracker.onChange(file, [doc.editLine(0, '@AutoEntity()')], undefined, doc.read);
    const [delta] = tracker.onSave(file, doc.read);
    assert.ok(delta);
    const learned = await learnDelta(taste, root, file, delta.before, delta.after);
    assert.ok(learned, 'learn-delta answered within the contract');
    promoted = learned.promoted[0]?.instruction;
  }
  assert.match(promoted ?? '', /@AutoEntity/);
  assert.match(fs.readFileSync(path.join(root, 'CLAUDE.md'), 'utf8'), /@AutoEntity/);

  // The toast's [Ignore] runs `taste disable`, which takes it back out.
  const rules = JSON.parse(fs.readFileSync(path.join(root, '.taste', 'rules.json'), 'utf8')) as { rules: { id: string }[] };
  const id = rules.rules[0]?.id as string;
  assert.equal((await run(taste, ['disable', id, '--quiet'], { cwd: root })).code, 0);
  // With nothing left to say, a CLAUDE.md that OpenTaste created is removed.
  const claude = path.join(root, 'CLAUDE.md');
  assert.doesNotMatch(fs.existsSync(claude) ? fs.readFileSync(claude, 'utf8') : '', /@AutoEntity/);
});
