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
    copyOnSelect.set(true); // user acts before hydrate resolves
    await copyOnSelect.hydrate();
    expect(get(copyOnSelect)).toBe(true);
  });
});
