import { strict as assert } from 'node:assert';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { test } from 'node:test';

import { learnDelta, parseLearned, resolveBinary, run } from '../binaryRunner';

// Byte for byte what `taste learn-delta` prints (crates/taste-cli/tests/cli.rs).
const PROMOTED =
  '{"promoted":[{"id":"arch-autoentity-entity-dfa0ba","category":"architecture","scope":"src/**/*.entity.ts",' +
  '"confidence":0.82,"occurrences":6,"enabled":true,"source":"ast-diff","instruction":"Use `@AutoEntity` instead of `@Entity`.",' +
  '"anti_pattern":"@Entity(\\"users\\")","preferred_pattern":"@AutoEntity()","updated_at":"2026-10-29T11:17:10.712411509Z"}],' +
  '"candidates":[]}\n';

test('a promoted rule parses with every TasteRule field', () => {
  const learned = parseLearned({ code: 0, stdout: PROMOTED });
  assert.ok(learned);
  assert.equal(learned.promoted.length, 1);
  assert.equal(learned.promoted[0]?.instruction, 'Use `@AutoEntity` instead of `@Entity`.');
  assert.equal(learned.promoted[0]?.anti_pattern, '@Entity("users")');
  assert.deepEqual(learned.candidates, []);
});

test('the learned-nothing answer is two empty arrays, not a failure', () => {
  assert.deepEqual(parseLearned({ code: 0, stdout: '{"promoted":[],"candidates":[]}\n' }), {
    promoted: [],
    candidates: [],
  });
});

test('a rule without patterns omits those keys and still parses', () => {
  const stdout =
    '{"promoted":[{"id":"x","category":"imports","scope":"**/*","confidence":0.95,"occurrences":1,' +
    '"enabled":true,"source":"mcp-feedback","instruction":"i","updated_at":"2026-10-29T00:00:00Z"}],' +
    '"candidates":[{"id":"y","confidence":0.45,"instruction":"j"}]}';
  const learned = parseLearned({ code: 0, stdout });
  assert.ok(learned);
  assert.equal(learned.promoted[0]?.anti_pattern, undefined);
  assert.deepEqual(learned.candidates, [{ id: 'y', confidence: 0.45, instruction: 'j' }]);
});

test('a non-zero exit with an empty stdout is a quiet failure', () => {
  assert.equal(parseLearned({ code: 1, stdout: '' }), undefined);
  assert.equal(parseLearned({ code: null, stdout: '' }), undefined);
  assert.equal(parseLearned({ code: 1, stdout: PROMOTED }), undefined, 'the exit code wins');
});

test('anything off-contract is rejected rather than half-trusted', () => {
  for (const stdout of [
    'not json',
    '[]',
    '{"promoted":[]}',
    '{"candidates":[]}',
    '{"promoted":[{"id":"x"}],"candidates":[]}',
    '{"promoted":[],"candidates":[{"id":"y","confidence":"high","instruction":"j"}]}',
  ]) {
    assert.equal(parseLearned({ code: 0, stdout }), undefined, stdout);
  }
});

test('the binary is found in the configured path, then the workspace, then PATH', () => {
  const present = new Set([
    '/proj/tools/taste',
    '/proj/node_modules/.bin/taste',
    '/usr/local/bin/taste',
  ]);
  const env = { platform: 'linux' as const, pathEnv: '/usr/bin:/usr/local/bin', exists: (f: string) => present.has(f) };
  assert.equal(resolveBinary('tools/taste', '/proj', env), '/proj/tools/taste');
  assert.equal(resolveBinary('/missing/taste', '/proj', env), undefined, 'a wrong setting is not silently ignored');
  assert.equal(resolveBinary('', '/proj', env), '/proj/node_modules/.bin/taste');
  present.delete('/proj/node_modules/.bin/taste');
  assert.equal(resolveBinary('', '/proj', env), '/usr/local/bin/taste');
  present.clear();
  assert.equal(resolveBinary('', '/proj', env), undefined);
});

test('on Windows the PATHEXT names and the semicolon PATH are used', () => {
  const present = new Set(['C:\\tools\\taste.exe', 'C:\\proj\\node_modules\\.bin\\taste.cmd']);
  const env = {
    platform: 'win32' as const,
    pathEnv: 'C:\\Windows;C:\\tools',
    pathExt: '.COM;.EXE;.CMD',
    exists: (f: string) => present.has(f),
  };
  assert.equal(resolveBinary('', 'C:\\proj', env), 'C:\\proj\\node_modules\\.bin\\taste.cmd');
  present.delete('C:\\proj\\node_modules\\.bin\\taste.cmd');
  assert.equal(resolveBinary('', 'C:\\proj', env), 'C:\\tools\\taste.exe');
});

/** A stand-in `taste` written in Node, so the process handling is real. */
function fakeBinary(script: string): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'opentaste-bin-'));
  const file = path.join(dir, 'fake.js');
  fs.writeFileSync(file, script);
  return file;
}

test('learnDelta sends the edit on stdin with --file and parses the answer', async (t) => {
  if (process.platform === 'win32') {
    t.skip('spawns a shebang script');
    return;
  }
  const script = fakeBinary(
    `#!${process.execPath}\n` +
      "let input='';process.stdin.on('data',d=>input+=d).on('end',()=>{" +
      "const {before,after}=JSON.parse(input);" +
      "const args=process.argv.slice(2).join(' ');" +
      "process.stdout.write(JSON.stringify({promoted:[],candidates:[{id:args+'|'+process.cwd(),confidence:0.45,instruction:before+'->'+after}]})+'\\n');});",
  );
  fs.chmodSync(script, 0o755);
  const cwd = fs.realpathSync(os.tmpdir());
  const learned = await learnDelta(script, cwd, 'src/a b.ts', 'x', 'y');
  assert.deepEqual(learned?.candidates, [
    { id: `learn-delta --file src/a b.ts|${cwd}`, confidence: 0.45, instruction: 'x->y' },
  ]);
});

test('a binary that hangs is killed at the timeout and reported as a failure', async (t) => {
  if (process.platform === 'win32') {
    t.skip('spawns a shebang script');
    return;
  }
  const script = fakeBinary(`#!${process.execPath}\nsetInterval(()=>{}, 1000);\n`);
  fs.chmodSync(script, 0o755);
  const started = Date.now();
  const result = await run(script, [], { cwd: os.tmpdir(), timeoutMs: 300 });
  assert.equal(result.timedOut, true);
  assert.equal(result.code, null);
  assert.ok(Date.now() - started < 5_000);
});

test('a binary that does not exist is a failed run, not an exception', async () => {
  const result = await run('/definitely/not/here/taste', ['status'], { cwd: os.tmpdir() });
  assert.equal(result.code, null);
  assert.equal(parseLearned(result), undefined);
});
