// A tiny text document that applies changes the way VS Code reports them:
// each range is in the document as it was before the change.

import type { ContentChange, Position, ReadLines } from '../deltaTracker';

export class FakeDocument {
  lines: string[];

  constructor(text: string) {
    this.lines = text.split('\n');
  }

  get text(): string {
    return this.lines.join('\n');
  }

  private offset(position: Position): number {
    let offset = 0;
    for (let line = 0; line < position.line; line++) {
      offset += (this.lines[line] ?? '').length + 1;
    }
    return offset + position.character;
  }

  /** Replace `start..end` with `text` and return the change event for it. */
  replace(start: Position, end: Position, text: string): ContentChange {
    const whole = this.text;
    const updated = whole.slice(0, this.offset(start)) + text + whole.slice(this.offset(end));
    this.lines = updated.split('\n');
    return { start, end, text };
  }

  /** Insert `text` at the start of `line`. */
  insertAt(line: number, text: string): ContentChange {
    const at = { line, character: 0 };
    return this.replace(at, at, text);
  }

  /** Delete whole lines `first..=last`. */
  deleteLines(first: number, last: number): ContentChange {
    return this.replace({ line: first, character: 0 }, { line: last + 1, character: 0 }, '');
  }

  /** Replace the whole of `line` with `text`. */
  editLine(line: number, text: string): ContentChange {
    return this.replace({ line, character: 0 }, { line, character: (this.lines[line] ?? '').length }, text);
  }

  readonly read: ReadLines = (start, end) => this.lines.slice(start, end + 1).join('\n');
}
