// Which files the extension may learn from.
//
// The transpiler's own output must never be learned from. Its paths come from
// `taste targets --json`, the list `config::Paths` owns in the Rust core, and
// are not repeated here. The extension also only tracks the script languages
// `taste learn-delta` can analyse.

/** The shape `taste targets --json` prints. */
export interface Targets {
  marker_targets: string[];
  cursor_rules_dir: string;
  cursor_rule_prefix: string;
  cursor_rule_extension: string;
}

/** Extensions the structural diff engine reads (`SupportedLanguage::is_script`). */
const SCRIPT_EXTENSIONS = ['.ts', '.tsx', '.mts', '.cts', '.js', '.jsx', '.mjs', '.cjs'];

/** Directories whose files are never the developer's own edits. */
const IGNORED_DIRS = ['node_modules', '.git', '.taste', 'dist', 'build', 'out', 'coverage', 'vendor', '.next'];

export function parseTargets(stdout: string): Targets | undefined {
  try {
    const value = JSON.parse(stdout) as Partial<Targets>;
    if (
      Array.isArray(value.marker_targets) &&
      value.marker_targets.every((t) => typeof t === 'string') &&
      typeof value.cursor_rules_dir === 'string' &&
      typeof value.cursor_rule_prefix === 'string' &&
      typeof value.cursor_rule_extension === 'string'
    ) {
      return value as Targets;
    }
  } catch {
    // Fall through: an unreadable answer means "do not learn".
  }
  return undefined;
}

function normalise(relativePath: string): string {
  return relativePath.replace(/\\/g, '/').replace(/^\.\//, '');
}

/** Whether `relativePath` (relative to the project root) is something `taste sync` writes. */
export function isGeneratedTarget(relativePath: string, targets: Targets): boolean {
  const path = normalise(relativePath);
  if (targets.marker_targets.some((target) => normalise(target) === path)) {
    return true;
  }
  const slash = path.lastIndexOf('/');
  const dir = slash === -1 ? '' : path.slice(0, slash);
  const name = path.slice(slash + 1);
  return (
    dir === normalise(targets.cursor_rules_dir) &&
    name.startsWith(targets.cursor_rule_prefix) &&
    name.endsWith(`.${targets.cursor_rule_extension}`)
  );
}

/** Whether edits to `relativePath` may teach OpenTaste anything. */
export function isLearnable(relativePath: string, targets: Targets): boolean {
  const path = normalise(relativePath);
  if (path.startsWith('../') || path.startsWith('/') || /^[a-zA-Z]:/.test(path)) {
    return false; // Outside the project root.
  }
  if (isGeneratedTarget(path, targets)) {
    return false;
  }
  if (path.split('/').some((part) => IGNORED_DIRS.includes(part))) {
    return false;
  }
  if (path.endsWith('.d.ts') || path.endsWith('.min.js')) {
    return false;
  }
  return SCRIPT_EXTENSIONS.some((ext) => path.toLowerCase().endsWith(ext));
}
