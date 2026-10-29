// The status bar item and the "learned" toast.
//
// Both stay out of the way: the item is a count, and the toast appears only
// when a rule was actually promoted. Nothing here ever opens a modal.

import * as vscode from 'vscode';

import type { TasteRule } from './binaryRunner';
import { learnedMessage } from './rules';

export const KEEP = 'Keep Rule';
export const IGNORE = 'Ignore';

export class StatusBar implements vscode.Disposable {
  private readonly item: vscode.StatusBarItem;

  constructor() {
    this.item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 10);
    this.item.command = 'opentaste.openDashboard';
    this.item.tooltip = 'OpenTaste: open the rule dashboard';
  }

  /** Show the count, or hide the item when there is no project to count. */
  update(enabledRules: number | undefined): void {
    if (enabledRules === undefined) {
      this.item.hide();
      return;
    }
    this.item.text = `$(sparkle) ${enabledRules} rule${enabledRules === 1 ? '' : 's'}`;
    this.item.show();
  }

  dispose(): void {
    this.item.dispose();
  }
}

/**
 * Offer a just-promoted rule. `[Keep Rule]` dismisses; `[Ignore]` calls
 * `onIgnore`, which disables the rule (it is never deleted from here).
 */
export async function announce(rule: TasteRule, onIgnore: (id: string) => Promise<void>): Promise<void> {
  const choice = await vscode.window.showInformationMessage(learnedMessage(rule.instruction), KEEP, IGNORE);
  if (choice === IGNORE) {
    await onIgnore(rule.id);
  }
}
