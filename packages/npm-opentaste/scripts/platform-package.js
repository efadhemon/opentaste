#!/usr/bin/env node
'use strict';

// Release helper, run by .github/workflows/release.yml.
//
//   platform-package.js platform <platform> <arch> <binary> <out-dir> <version>
//     Lay out one platform package: package.json with os/cpu, bin/taste[.exe].
//   platform-package.js wrapper <wrapper-dir> <version>
//     Stamp the version into the wrapper and list every platform package as
//     an optional dependency at exactly that version.

const fs = require('node:fs');
const path = require('node:path');
const { TARGETS, packageName, binaryName } = require('../lib/platform.js');

function platformManifest(platform, arch, version) {
  return {
    name: packageName(platform, arch),
    version,
    description: `The OpenTaste \`taste\` binary for ${platform}-${arch}.`,
    license: 'MIT',
    repository: { type: 'git', url: 'https://github.com/efadhemon/opentaste' },
    os: [platform],
    cpu: [arch],
    files: ['bin'],
  };
}

function wrapperManifest(manifest, version) {
  const optionalDependencies = {};
  for (const target of TARGETS) {
    optionalDependencies[packageName(target.platform, target.arch)] = version;
  }
  return { ...manifest, version, optionalDependencies };
}

function main(argv) {
  const [mode, ...rest] = argv;
  if (mode === 'platform') {
    const [platform, arch, binary, outDir, version] = rest;
    const target = TARGETS.find((t) => t.platform === platform && t.arch === arch);
    if (!target || !binary || !outDir || !version) {
      throw new Error('usage: platform <platform> <arch> <binary> <out-dir> <version>');
    }
    fs.mkdirSync(path.join(outDir, 'bin'), { recursive: true });
    const dest = path.join(outDir, 'bin', binaryName(platform));
    fs.copyFileSync(binary, dest);
    fs.chmodSync(dest, 0o755);
    fs.writeFileSync(
      path.join(outDir, 'package.json'),
      `${JSON.stringify(platformManifest(platform, arch, version), null, 2)}\n`,
    );
    return;
  }
  if (mode === 'wrapper') {
    const [dir, version] = rest;
    if (!dir || !version) {
      throw new Error('usage: wrapper <wrapper-dir> <version>');
    }
    const file = path.join(dir, 'package.json');
    const manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
    fs.writeFileSync(file, `${JSON.stringify(wrapperManifest(manifest, version), null, 2)}\n`);
    return;
  }
  throw new Error('usage: platform-package.js <platform|wrapper> ...');
}

if (require.main === module) {
  try {
    main(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`${err.message}\n`);
    process.exit(1);
  }
}

module.exports = { platformManifest, wrapperManifest, main };
