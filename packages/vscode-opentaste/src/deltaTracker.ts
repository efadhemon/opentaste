// Detect "an AI wrote a block, then a human corrected it, then saved".
//
// A single change that inserts at least four lines opens an AiBurstWindow over
// the inserted lines. Any smaller edit inside that window marks it corrected. On
// save, a corrected window younger than 180 seconds yields a delta: the
// snapshot taken when the block landed, and the same lines as saved.
//
// "Smaller" means "not itself a burst", with no character limit. SPEC.md once
// said 1–15 characters, but most corrections the detectors learn from arrive
// as one longer edit: a pasted import path, a rename, a "convert to arrow
// function" code action. The detectors decide what counts; this only decides
// which lines to hand them.
//
// This module does not import `vscode`. The extension adapts editor events to
// the small types below, which is what lets the three hazards be tested by
// replaying exact event sequences:
//
// - Undo and redo reinsert whole blocks and look exactly like bursts. They
//   never open a window, and they close every window in the document, since
//   the text a window tracks may be what was just undone.
// - OpenTaste's own sync rewrites CLAUDE.md, AGENTS.md and the rest. Only
//   files `isLearnable` accepts are tracked at all.
// - An edit above a window shifts its lines. Every change moves the windows
//   below it, or the captured "after" would be a different block entirely.

export interface Position {
  line: number;
  character: number;
}

/** One replaced range, in the document as it was before this change. */
export interface ContentChange {
  start: Position;
  end: Position;
  text: string;
}

export type ChangeReason = 'undo' | 'redo' | undefined;

/** Lines `start..=end` of the document as it is now. */
export type ReadLines = (start: number, end: number) => string;

export interface AiBurstWindow {
  filePath: string;
  /** First line of the block, inclusive, in the current document. */
  startLine: number;
  /** Last line of the block, inclusive, in the current document. */
  endLine: number;
  aiSnapshotText: string;
  timestamp: number;
  corrected: boolean;
}

export interface Delta {
  filePath: string;
  before: string;
  after: string;
}

export interface TrackerOptions {
  now?: () => number;
  /** Relative paths the tracker may learn from. */
  isLearnable: (filePath: string) => boolean;
  lifetimeMs?: number;
  maxWindowsPerFile?: number;
  minBurstLines?: number;
}

export const DEFAULT_LIFETIME_MS = 180_000;
export const DEFAULT_MAX_WINDOWS = 8;
export const DEFAULT_MIN_BURST_LINES = 4;

/** Lines a change inserts: "a\nb\nc\nd" and "a\nb\nc\nd\n" are both four. */
export function insertedLines(text: string): number {
  if (text.length === 0) {
    return 0;
  }
  const lines = text.split(/\r\n|\r|\n/);
  return text.endsWith('\n') || text.endsWith('\r') ? lines.length - 1 : lines.length;
}

/** How far a change moves the lines after it. */
function lineDelta(change: ContentChange): number {
  const added = change.text.split(/\r\n|\r|\n/).length - 1;
  const removed = change.end.line - change.start.line;
  return added - removed;
}

/**
 * Whether `change` leaves line `startLine` and everything after it intact,
 * only moving it. Besides a change that ends on an earlier line, that covers
 * whole lines inserted or deleted right up to column 0 of `startLine`: adding
 * an import directly above a block pushes the block down without touching it.
 */
function endsAbove(change: ContentChange, startLine: number): boolean {
  if (change.end.line < startLine) {
    return true;
  }
  if (change.end.line !== startLine || change.end.character !== 0) {
    return false;
  }
  if (change.text === '') {
    return change.start.character === 0; // Whole lines deleted above.
  }
  return /(\r\n|\r|\n)$/.test(change.text) && change.start.character === 0;
}

export class DeltaTracker {
  private readonly windows = new Map<string, AiBurstWindow[]>();
  private readonly now: () => number;
  private readonly isLearnable: (filePath: string) => boolean;
  private readonly lifetimeMs: number;
  private readonly maxWindows: number;
  private readonly minBurstLines: number;

  constructor(options: TrackerOptions) {
    this.now = options.now ?? Date.now;
    this.isLearnable = options.isLearnable;
    this.lifetimeMs = options.lifetimeMs ?? DEFAULT_LIFETIME_MS;
    this.maxWindows = options.maxWindowsPerFile ?? DEFAULT_MAX_WINDOWS;
    this.minBurstLines = options.minBurstLines ?? DEFAULT_MIN_BURST_LINES;
  }

  /** Open windows for `filePath`, oldest first. */
  windowsFor(filePath: string): readonly AiBurstWindow[] {
    return this.windows.get(filePath) ?? [];
  }

  /**
   * Fold one document change event into the open windows. `readLines` reads
   * the document after the change, as `onDidChangeTextDocument` sees it.
   */
  onChange(filePath: string, changes: readonly ContentChange[], reason: ChangeReason, readLines: ReadLines): void {
    if (!this.isLearnable(filePath)) {
      return;
    }
    if (reason === 'undo' || reason === 'redo') {
      this.windows.delete(filePath);
      return;
    }
    let windows = this.windows.get(filePath) ?? [];

    // VS Code lists a transaction's changes against the document before any
    // of them applied. Applying them bottom-up keeps each one's line numbers
    // valid for the windows it affects.
    const ordered = [...changes].sort((a, b) => b.start.line - a.start.line || b.start.character - a.start.character);
    for (const change of ordered) {
      windows = this.applyChange(windows, change);
    }

    const burst = changes.length === 1 ? changes[0] : undefined;
    if (burst && insertedLines(burst.text) >= this.minBurstLines) {
      const startLine = burst.start.line;
      const endLine = startLine + insertedLines(burst.text) - 1;
      // A new burst over an old window replaces it: the old block is gone.
      windows = windows.filter((w) => w.endLine < startLine || w.startLine > endLine);
      windows.push({
        filePath,
        startLine,
        endLine,
        aiSnapshotText: readLines(startLine, endLine),
        timestamp: this.now(),
        corrected: false,
      });
      while (windows.length > this.maxWindows) {
        windows.shift();
      }
    }

    this.store(filePath, windows);
  }

  private applyChange(windows: AiBurstWindow[], change: ContentChange): AiBurstWindow[] {
    const delta = lineDelta(change);
    const isBurst = insertedLines(change.text) >= this.minBurstLines;
    const kept: AiBurstWindow[] = [];
    for (const window of windows) {
      if (endsAbove(change, window.startLine)) {
        // Entirely above: the window moves with the text.
        window.startLine += delta;
        window.endLine += delta;
        kept.push(window);
        continue;
      }
      if (change.start.line > window.endLine) {
        kept.push(window);
        continue;
      }
      // The change touches the window. One that starts above it or is itself
      // a burst makes the tracked range ambiguous, and an ambiguous window
      // would teach a wrong rule, so it is dropped.
      if (isBurst || change.start.line < window.startLine) {
        continue;
      }
      window.endLine += delta;
      if (window.endLine < window.startLine) {
        continue; // The whole block was deleted.
      }
      window.corrected = true;
      kept.push(window);
    }
    return kept;
  }

  /**
   * The deltas a save of `filePath` produces. Corrected, live windows fire
   * once and close; uncorrected ones stay open until they expire.
   */
  onSave(filePath: string, readLines: ReadLines): Delta[] {
    const windows = this.windows.get(filePath);
    if (!windows) {
      return [];
    }
    const now = this.now();
    const deltas: Delta[] = [];
    const remaining: AiBurstWindow[] = [];
    for (const window of windows) {
      if (now - window.timestamp > this.lifetimeMs) {
        continue;
      }
      if (!window.corrected) {
        remaining.push(window);
        continue;
      }
      const after = readLines(window.startLine, window.endLine);
      if (after !== window.aiSnapshotText) {
        deltas.push({ filePath, before: window.aiSnapshotText, after });
      }
    }
    this.store(filePath, remaining);
    return deltas;
  }

  /** Drop windows past their lifetime, so an idle file holds nothing stale. */
  expire(): void {
    const now = this.now();
    for (const [filePath, windows] of this.windows) {
      this.store(
        filePath,
        windows.filter((w) => now - w.timestamp <= this.lifetimeMs),
      );
    }
  }

  /** Forget a document that was closed or renamed. */
  forget(filePath: string): void {
    this.windows.delete(filePath);
  }

  private store(filePath: string, windows: AiBurstWindow[]): void {
    if (windows.length === 0) {
      this.windows.delete(filePath);
    } else {
      this.windows.set(filePath, windows);
    }
  }
}
