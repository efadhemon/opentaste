// Read `.taste/rules.json` for the status bar. Display only: the binary owns
// every decision about what a rule is.

/** How many confirmed rules are switched on, or `undefined` if unreadable. */
export function countEnabledRules(rulesJson: string): number | undefined {
  try {
    const value = JSON.parse(rulesJson) as { rules?: unknown };
    if (!Array.isArray(value.rules)) {
      return undefined;
    }
    return value.rules.filter(
      (rule) => typeof rule === 'object' && rule !== null && (rule as { enabled?: unknown }).enabled === true,
    ).length;
  } catch {
    return undefined;
  }
}

/** The toast text from SPEC.md section 9. */
export function learnedMessage(instruction: string): string {
  return `✨ OpenTaste learned: "${instruction}"`;
}
