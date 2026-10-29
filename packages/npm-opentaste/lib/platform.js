'use strict';

// Which prebuilt binary package serves this machine.
//
// The binaries ship as one npm package per platform, listed as
// optionalDependencies of the wrapper (added by the release job), each with
// `os` and `cpu` fields so npm installs only the matching one. There is no
// postinstall download: that breaks under --ignore-scripts and behind proxies,
// and a tool that promises to work offline should not phone home to install.

/** npm scope and wrapper name. Change them here and nowhere else. */
const SCOPE = '@opentaste';
const WRAPPER = `${SCOPE}/cli`;

/** Every supported target: npm's platform/arch, Rust's triple, and runner. */
const TARGETS = [
  { platform: 'linux', arch: 'x64', triple: 'x86_64-unknown-linux-gnu' },
  { platform: 'linux', arch: 'arm64', triple: 'aarch64-unknown-linux-gnu' },
  { platform: 'darwin', arch: 'x64', triple: 'x86_64-apple-darwin' },
  { platform: 'darwin', arch: 'arm64', triple: 'aarch64-apple-darwin' },
  { platform: 'win32', arch: 'x64', triple: 'x86_64-pc-windows-msvc' },
];

function packageName(platform, arch) {
  return `${SCOPE}/cli-${platform}-${arch}`;
}

function binaryName(platform) {
  return platform === 'win32' ? 'taste.exe' : 'taste';
}

/**
 * Whether this Linux uses musl rather than glibc. The Linux binaries link
 * glibc and will not start on Alpine, so say so instead of failing oddly.
 * `report` is injectable for tests.
 */
function isMusl(report) {
  const header = report && report.header;
  return Boolean(header) && !header.glibcVersionRuntime;
}

/**
 * The package and binary for this machine, or an `error` saying why there is
 * none. `report` is `process.report.getReport()` on Linux.
 */
function resolveTarget(platform, arch, report) {
  const target = TARGETS.find((t) => t.platform === platform && t.arch === arch);
  if (!target) {
    const supported = TARGETS.map((t) => `${t.platform}-${t.arch}`).join(', ');
    return {
      error:
        `OpenTaste has no prebuilt binary for ${platform}-${arch}. ` +
        `Supported: ${supported}. You can build it with \`cargo install --path crates/taste-cli\`.`,
    };
  }
  if (platform === 'linux' && isMusl(report)) {
    return {
      error:
        'OpenTaste\'s Linux binaries need glibc, and this system uses musl (Alpine?). ' +
        'Build it with `cargo install --path crates/taste-cli` instead.',
    };
  }
  return {
    target,
    packageName: packageName(platform, arch),
    binary: binaryName(platform),
  };
}

module.exports = { SCOPE, WRAPPER, TARGETS, packageName, binaryName, isMusl, resolveTarget };
