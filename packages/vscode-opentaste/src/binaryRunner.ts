// Talk to the local `taste` binary.
//
// `taste learn-delta` is the contract fixed in plans/PHASE-5.md step 3: the
// edit goes in on stdin as `{"before", "after"}`, and one line comes back:
// `{"promoted": TasteRule[], "candidates": {id, confidence, instruction}[]}`.
// A failure is an empty stdout and a non-zero exit, and must stay silent.
//
// No `vscode` import here either, so parsing, resolution and the process
// handling are tested in plain Node.

import { spawn as nodeSpawn, type ChildProcess, type SpawnOptions } from 'node:child_process';
import * as path from 'node:path';

/** A confirmed rule, exactly as `.taste/rules.json` stores it. */
export interface TasteRule {
  id: string;
  category: string;
  scope: string;
  confidence: number;
  occurrences: number;
  enabled: boolean;
  source: string;
  instruction: string;
  /** Absent when the rule has none, as in `rules.json`. */
  anti_pattern?: string;
  /** Absent when the rule has none, as in `rules.json`. */
  preferred_pattern?: string;
  updated_at: string;
}

export interface CandidateSummary {
  id: string;
  confidence: number;
  instruction: string;
}

export interface Learned {
  promoted: TasteRule[];
  candidates: CandidateSummary[];
}

export interface RunResult {
  code: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

/** Long enough for a cold SQLite open on a slow disk, short enough not to pile up. */
export const DEFAULT_TIMEOUT_MS = 10_000;

function isRule(value: unknown): value is TasteRule {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const rule = value as Record<string, unknown>;
  const optionalString = (key: string) => rule[key] === undefined || typeof rule[key] === 'string';
  return (
    typeof rule.id === 'string' &&
    typeof rule.category === 'string' &&
    typeof rule.scope === 'string' &&
    typeof rule.confidence === 'number' &&
    typeof rule.occurrences === 'number' &&
    typeof rule.enabled === 'boolean' &&
    typeof rule.source === 'string' &&
    typeof rule.instruction === 'string' &&
    typeof rule.updated_at === 'string' &&
    optionalString('anti_pattern') &&
    optionalString('preferred_pattern')
  );
}

function isCandidate(value: unknown): value is CandidateSummary {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.id === 'string' &&
    typeof candidate.confidence === 'number' &&
    typeof candidate.instruction === 'string'
  );
}

/**
 * Parse one `learn-delta` answer. `undefined` for a failed run or anything
 * that does not match the contract: the caller stays silent either way.
 */
export function parseLearned(result: Pick<RunResult, 'code' | 'stdout'>): Learned | undefined {
  if (result.code !== 0 || result.stdout.trim() === '') {
    return undefined;
  }
  let value: unknown;
  try {
    value = JSON.parse(result.stdout);
  } catch {
    return undefined;
  }
  if (typeof value !== 'object' || value === null) {
    return undefined;
  }
  const { promoted, candidates } = value as Record<string, unknown>;
  if (!Array.isArray(promoted) || !Array.isArray(candidates)) {
    return undefined;
  }
  if (!promoted.every(isRule) || !candidates.every(isCandidate)) {
    return undefined;
  }
  return { promoted, candidates };
}

export interface ResolveEnv {
  platform: NodeJS.Platform;
  /** The `PATH` (or `Path`) variable. */
  pathEnv: string | undefined;
  /** Windows' `PATHEXT`. */
  pathExt?: string | undefined;
  exists: (file: string) => boolean;
}

/**
 * Where `taste` is, in order: the configured path, the workspace's own
 * `node_modules/.bin` (an `npm install @opentaste/cli`), then `PATH`.
 * `undefined` when it is nowhere, which the extension treats as "stay quiet".
 */
export function resolveBinary(configured: string, workspaceRoot: string, env: ResolveEnv): string | undefined {
  const pathApi = env.platform === 'win32' ? path.win32 : path.posix;
  if (configured.trim() !== '') {
    const candidate = pathApi.isAbsolute(configured) ? configured : pathApi.join(workspaceRoot, configured);
    return env.exists(candidate) ? candidate : undefined;
  }
  const names =
    env.platform === 'win32'
      ? (env.pathExt ?? '.EXE;.CMD;.BAT')
          .split(';')
          .filter(Boolean)
          .map((ext) => `taste${ext.toLowerCase()}`)
      : ['taste'];
  const local = names
    .map((name) => pathApi.join(workspaceRoot, 'node_modules', '.bin', name))
    .find((file) => env.exists(file));
  if (local) {
    return local;
  }
  const separator = env.platform === 'win32' ? ';' : ':';
  for (const dir of (env.pathEnv ?? '').split(separator).filter(Boolean)) {
    const found = names.map((name) => pathApi.join(dir, name)).find((file) => env.exists(file));
    if (found) {
      return found;
    }
  }
  return undefined;
}

export type Spawn = (command: string, args: readonly string[], options: SpawnOptions) => ChildProcess;

export interface RunOptions {
  cwd: string;
  stdin?: string;
  timeoutMs?: number;
  spawn?: Spawn;
}

/**
 * Run `binary` with `args`, feeding `stdin`, and collect its output. A run
 * past `timeoutMs` is killed, so a hung binary cannot leak a process per save.
 * Never rejects: a failure to start is reported as a failed run.
 */
export function run(binary: string, args: readonly string[], options: RunOptions): Promise<RunResult> {
  const spawn = options.spawn ?? nodeSpawn;
  const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  return new Promise((resolve) => {
    let child: ChildProcess;
    try {
      // `.cmd` shims from npm need a shell on Windows; a real binary does not.
      const shell = process.platform === 'win32' && /\.(cmd|bat)$/i.test(binary);
      child = spawn(binary, args, { cwd: options.cwd, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true, shell });
    } catch (err) {
      resolve({ code: null, stdout: '', stderr: String(err), timedOut: false });
      return;
    }
    let stdout = '';
    let stderr = '';
    let timedOut = false;
    let settled = false;
    const finish = (code: number | null) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timer);
      resolve({ code, stdout, stderr, timedOut });
    };
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill();
      finish(null);
    }, timeoutMs);
    child.stdout?.setEncoding('utf8').on('data', (chunk: string) => (stdout += chunk));
    child.stderr?.setEncoding('utf8').on('data', (chunk: string) => (stderr += chunk));
    child.on('error', (err) => {
      stderr += String(err);
      finish(null);
    });
    child.on('close', (code) => finish(code));
    child.stdin?.on('error', () => {
      // The child exited before reading stdin; its exit code says enough.
    });
    child.stdin?.end(options.stdin ?? '');
  });
}

/** `taste learn-delta --file <relative>` for one corrected edit. */
export async function learnDelta(
  binary: string,
  cwd: string,
  relativeFile: string,
  before: string,
  after: string,
  options: Omit<RunOptions, 'cwd' | 'stdin'> = {},
): Promise<Learned | undefined> {
  const result = await run(binary, ['learn-delta', '--file', relativeFile], {
    ...options,
    cwd,
    stdin: JSON.stringify({ before, after }),
  });
  return parseLearned(result);
}
