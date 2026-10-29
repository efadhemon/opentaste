#!/usr/bin/env node
'use strict';

// Run the prebuilt `taste` binary for this platform, transparently: argv,
// stdin, stdout, stderr, the exit code and a terminating signal all pass
// through. This script writes nothing to stdout itself, so `taste mcp`
// works as an MCP server through it.

const { spawnSync } = require('node:child_process');
const path = require('node:path');
const { resolveTarget } = require('../lib/platform.js');

const report = process.platform === 'linux' && process.report ? process.report.getReport() : undefined;
const resolved = resolveTarget(process.platform, process.arch, report);
if (resolved.error) {
  process.stderr.write(`taste: ${resolved.error}\n`);
  process.exit(1);
}

let binary;
try {
  const manifest = require.resolve(`${resolved.packageName}/package.json`);
  binary = path.join(path.dirname(manifest), 'bin', resolved.binary);
} catch {
  process.stderr.write(
    `taste: ${resolved.packageName} is not installed. It is an optional dependency, so it is ` +
      'skipped by --no-optional or --omit=optional; reinstall without that flag.\n',
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: 'inherit', windowsHide: true });
if (result.error) {
  process.stderr.write(`taste: could not run ${binary}: ${result.error.message}\n`);
  process.exit(1);
}
if (result.signal) {
  process.kill(process.pid, result.signal);
}
process.exit(result.status === null ? 1 : result.status);
