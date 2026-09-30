import { describe, expect, it } from 'vitest';
import { formatResults, isValidRegex, HIGHLIGHT_LIMIT } from './terminalSearch';

describe('formatResults', () => {
  it('shows the current match and the total', () => {
    expect(formatResults({ resultIndex: 2, resultCount: 17 })).toBe('3 / 17');
  });

  it('shows only the total before a current match is picked', () => {
    expect(formatResults({ resultIndex: -1, resultCount: 5 })).toBe('5');
  });

  it('says so when nothing matches, and caps past the highlight limit', () => {
    expect(formatResults({ resultIndex: -1, resultCount: 0 })).toBe('No results');
    expect(formatResults({ resultIndex: -1, resultCount: HIGHLIGHT_LIMIT + 1 })).toBe(
      `${HIGHLIGHT_LIMIT}+`
    );
  });
});

describe('isValidRegex', () => {
  it('accepts a pattern and rejects a broken one', () => {
    expect(isValidRegex('ERR(OR)?\\s+\\d+')).toBe(true);
    expect(isValidRegex('(unclosed')).toBe(false);
    expect(isValidRegex('[')).toBe(false);
  });
});
