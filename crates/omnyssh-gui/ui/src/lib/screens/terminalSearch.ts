// The terminal search bar's pure parts: how the result count reads, and whether a regex
// the user typed can be compiled. The search itself is @xterm/addon-search's.

/** The most matches the addon highlights; past it the count reads `1000+`. */
export const HIGHLIGHT_LIMIT = 1000;

/** `3 / 17`, `5` before a current match is picked, `No results`, or `1000+`. */
export function formatResults(r: { resultIndex: number; resultCount: number }): string {
  if (r.resultCount === 0) return 'No results';
  if (r.resultCount > HIGHLIGHT_LIMIT) return `${HIGHLIGHT_LIMIT}+`;
  if (r.resultIndex < 0) return String(r.resultCount);
  return `${r.resultIndex + 1} / ${r.resultCount}`;
}

/** Whether `pattern` compiles as a JavaScript regex (what the addon builds). */
export function isValidRegex(pattern: string): boolean {
  try {
    new RegExp(pattern);
    return true;
  } catch {
    return false;
  }
}
