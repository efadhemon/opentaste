'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const { TARGETS, resolveTarget, isMusl } = require('../lib/platform.js');
const { platformManifest, wrapperManifest, main } = require('../scripts/platform-package.js');

const glibc = { header: { glibcVersionRuntime: '2.39' } };

test('every supported platform and arch resolves to its own package and binary', () => {
  const expected = {
    'linux-x64': ['@opentaste/cli-linux-x64', 'taste'],
    'linux-arm64': ['@opentaste/cli-linux-arm64', 'taste'],
    'darwin-x64': ['@opentaste/cli-darwin-x64', 'taste'],
    'darwin-arm64': ['@opentaste/cli-darwin-arm64', 'taste'],
    'win32-x64': ['@opentaste/cli-win32-x64', 'taste.exe'],
  };
  assert.equal(TARGETS.length, Object.keys(expected).length);
  for (const [key, [pkg, binary]] of Object.entries(expected)) {
    const [platform, arch] = key.split('-');
    const resolved = resolveTarget(platform, arch, platform === 'linux' ? glibc : undefined);
    assert.equal(resolved.error, undefined, key);
    assert.equal(resolved.packageName, pkg, key);
    assert.equal(resolved.binary, binary, key);
  }
});

test('an unsupported platform names what is supported and how to build', () => {
  for (const [platform, arch] of [['freebsd', 'x64'], ['win32', 'arm64'], ['linux', 'ia32']]) {
    const resolved = resolveTarget(platform, arch, glibc);
    assert.match(resolved.error, new RegExp(`${platform}-${arch}`));
    assert.match(resolved.error, /linux-x64/);
    assert.match(resolved.error, /cargo install/);
  }
});

test('musl Linux is refused with a reason rather than a confusing exec failure', () => {
  assert.equal(isMusl({ header: {} }), true);
  assert.equal(isMusl(glibc), false);
  assert.equal(isMusl(undefined), false);
  assert.match(resolveTarget('linux', 'x64', { header: {} }).error, /musl/);
});

test('a platform package declares os and cpu so npm installs only the matching one', () => {
  const manifest = platformManifest('darwin', 'arm64', '1.2.3');
  assert.deepEqual(manifest.os, ['darwin']);
  assert.deepEqual(manifest.cpu, ['arm64']);
  assert.equal(manifest.name, '@opentaste/cli-darwin-arm64');
  assert.equal(manifest.version, '1.2.3');
  assert.equal(manifest.scripts, undefined, 'no install scripts, ever');
});

test('the wrapper lists every platform package at exactly the release version', () => {
  const wrapper = wrapperManifest({ name: '@opentaste/cli', version: '0.0.0', bin: { taste: 'bin/taste.js' } }, '1.2.3');
  assert.equal(wrapper.version, '1.2.3');
  assert.equal(Object.keys(wrapper.optionalDependencies).length, TARGETS.length);
  for (const version of Object.values(wrapper.optionalDependencies)) {
    assert.equal(version, '1.2.3');
  }
  assert.equal(wrapper.postinstall, undefined);
});

test('the platform command lays out an installable package', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'opentaste-pkg-'));
  const binary = path.join(dir, 'built-taste');
  fs.writeFileSync(binary, 'binary');
  main(['platform', 'linux', 'x64', binary, path.join(dir, 'out'), '0.1.0']);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'out', 'package.json'), 'utf8'));
  assert.equal(manifest.name, '@opentaste/cli-linux-x64');
  assert.equal(fs.readFileSync(path.join(dir, 'out', 'bin', 'taste'), 'utf8'), 'binary');
});

test('the wrapper script passes argv, stdio and the exit code through, and prints nothing itself', () => {
  // Stand up a fake platform package next to the wrapper, the way npm would.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opentaste-wrap-'));
  const resolved = resolveTarget(process.platform, process.arch, process.report && process.report.getReport());
  if (resolved.error) {
    return; // This runner has no supported binary; the resolution tests cover it.
  }
  const wrapper = path.join(root, 'node_modules', '@opentaste', 'cli');
  fs.mkdirSync(path.join(wrapper, 'bin'), { recursive: true });
  fs.mkdirSync(path.join(wrapper, 'lib'), { recursive: true });
  fs.copyFileSync(path.join(__dirname, '..', 'bin', 'taste.js'), path.join(wrapper, 'bin', 'taste.js'));
  fs.copyFileSync(path.join(__dirname, '..', 'lib', 'platform.js'), path.join(wrapper, 'lib', 'platform.js'));

  const platformDir = path.join(root, 'node_modules', ...resolved.packageName.split('/'));
  fs.mkdirSync(path.join(platformDir, 'bin'), { recursive: true });
  fs.writeFileSync(path.join(platformDir, 'package.json'), '{"name":"fake"}');
  // A stand-in binary: node re-running a tiny script that echoes argv and stdin.
  const fake = path.join(platformDir, 'bin', resolved.binary);
  const echo = path.join(root, 'echo.js');
  fs.writeFileSync(
    echo,
    "let input='';process.stdin.on('data',d=>input+=d).on('end',()=>{process.stdout.write(JSON.stringify({args:process.argv.slice(2),input}));process.exit(7);});",
  );
  if (process.platform === 'win32') {
    return; // A .cmd shim cannot stand in for taste.exe; the logic is the same.
  }
  fs.writeFileSync(fake, `#!/bin/sh\nexec "${process.execPath}" "${echo}" "$@"\n`);
  fs.chmodSync(fake, 0o755);

  const result = spawnSync(process.execPath, [path.join(wrapper, 'bin', 'taste.js'), 'learn-delta', '--file', 'a b.ts'], {
    input: 'payload',
    encoding: 'utf8',
  });
  assert.equal(result.status, 7, result.stderr);
  assert.deepEqual(JSON.parse(result.stdout), { args: ['learn-delta', '--file', 'a b.ts'], input: 'payload' });
  assert.equal(result.stderr, '');
});
