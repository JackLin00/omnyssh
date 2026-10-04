// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

// The two mouse-clipboard switches persist like the other UI prefs: a fake Tauri store,
// a fresh module per test to reset the singletons, and the localStorage mirror.
const backend = { get: vi.fn(), set: vi.fn(), save: vi.fn() };
vi.mock('@tauri-apps/plugin-store', () => ({ load: vi.fn(async () => backend) }));

async function fresh() {
  vi.resetModules();
  return await import('./terminalPrefs');
}

describe('terminal mouse-clipboard prefs', () => {
  beforeEach(() => {
    localStorage.clear();
    backend.get.mockReset();
    backend.set.mockReset().mockResolvedValue(undefined);
    backend.save.mockReset().mockResolvedValue(undefined);
  });

  it('both default to on', async () => {
    const { copyOnSelect, rightClickPaste } = await fresh();
    expect(get(copyOnSelect)).toBe(true);
    expect(get(rightClickPaste)).toBe(true);
  });

  it('a toggle is mirrored to localStorage and survives a reload', async () => {
    let prefs = await fresh();
    prefs.copyOnSelect.toggle();
    expect(get(prefs.copyOnSelect)).toBe(false);
    expect(localStorage.getItem('omnyssh-copy-on-select')).toBe('false');
    prefs = await fresh();
    expect(get(prefs.copyOnSelect)).toBe(false);
    // The other switch is independent.
    expect(get(prefs.rightClickPaste)).toBe(true);
  });

  it('writes the canonical tauri-plugin-store on a user flip', async () => {
    const { rightClickPaste } = await fresh();
    rightClickPaste.set(false);
    await vi.waitFor(() => {
      expect(backend.set).toHaveBeenCalledWith('rightClickPaste', false);
      expect(backend.save).toHaveBeenCalled();
    });
  });

  it('hydrate applies the stored value without clobbering a fresh user flip', async () => {
    backend.get.mockResolvedValue(false);
    const { copyOnSelect, rightClickPaste } = await fresh();
    await rightClickPaste.hydrate();
    expect(get(rightClickPaste)).toBe(false);
    copyOnSelect.set(true); // user acts before hydrate runs
    await copyOnSelect.hydrate();
    expect(get(copyOnSelect)).toBe(true);
  });
});

// The terminal font size: a number, clamped to 8-32, shared by every SSH and serial tab.
// Same persistence shape as the flags above (localStorage mirror + tauri-plugin-store).
describe('terminal font size', () => {
  beforeEach(() => {
    localStorage.clear();
    backend.get.mockReset();
    backend.set.mockReset().mockResolvedValue(undefined);
    backend.save.mockReset().mockResolvedValue(undefined);
  });

  it('defaults to 13', async () => {
    const { terminalFontSize } = await fresh();
    expect(get(terminalFontSize)).toBe(13);
  });

  it('set() clamps to the 8-32 range, and a non-number falls back to 13', async () => {
    const { terminalFontSize } = await fresh();
    terminalFontSize.set(40);
    expect(get(terminalFontSize)).toBe(32);
    terminalFontSize.set(2);
    expect(get(terminalFontSize)).toBe(8);
    terminalFontSize.set(NaN);
    expect(get(terminalFontSize)).toBe(13);
  });

  it('step(+1) and step(-1) move within the range', async () => {
    const { terminalFontSize } = await fresh();
    terminalFontSize.step(1);
    expect(get(terminalFontSize)).toBe(14);
    terminalFontSize.step(-1);
    expect(get(terminalFontSize)).toBe(13);
  });

  it('reset() returns to 13', async () => {
    const { terminalFontSize } = await fresh();
    terminalFontSize.set(20);
    terminalFontSize.reset();
    expect(get(terminalFontSize)).toBe(13);
  });

  it('a change is mirrored to localStorage and survives a reload', async () => {
    let prefs = await fresh();
    prefs.terminalFontSize.set(18);
    expect(localStorage.getItem('omnyssh-terminal-font-size')).toBe('18');
    prefs = await fresh();
    expect(get(prefs.terminalFontSize)).toBe(18);
  });

  it('hydrate applies the stored value without clobbering a fresh user edit', async () => {
    backend.get.mockResolvedValue(20);
    const { terminalFontSize } = await fresh();
    terminalFontSize.set(16); // user acts before hydrate runs
    await terminalFontSize.hydrate();
    expect(get(terminalFontSize)).toBe(16);
  });
});
