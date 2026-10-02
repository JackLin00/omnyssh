// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

// Persists like the other terminal-chord prefs: a fake Tauri store, a fresh module per
// test to reset the singletons, and the localStorage mirror (terminalPrefs.test.ts).
// `$lib/ipc/commands` is mocked too, so `applyGlobalHotkey` can be checked without a
// real backend.
const backend = { get: vi.fn(), set: vi.fn(), save: vi.fn() };
vi.mock('@tauri-apps/plugin-store', () => ({ load: vi.fn(async () => backend) }));

const setGlobalHotkey = vi.fn();
vi.mock('$lib/ipc/commands', () => ({ setGlobalHotkey }));

async function fresh() {
  vi.resetModules();
  return await import('./globalHotkey');
}

describe('chordToAccelerator', () => {
  it('converts a chord into the accelerator syntax the plugin parses', async () => {
    const { chordToAccelerator } = await fresh();
    expect(chordToAccelerator('Ctrl+Space')).toBe('Control+Space');
    expect(chordToAccelerator('Meta+K')).toBe('Super+K');
    expect(chordToAccelerator('Ctrl+Alt+Backquote')).toBe('Control+Alt+Backquote');
  });

  it('is null for a key the plugin has no entry for', async () => {
    const { chordToAccelerator } = await fresh();
    expect(chordToAccelerator('Ctrl+ContextMenu')).toBeNull();
  });
});

describe('globalHotkey', () => {
  beforeEach(() => {
    localStorage.clear();
    backend.get.mockReset();
    backend.set.mockReset().mockResolvedValue(undefined);
    backend.save.mockReset().mockResolvedValue(undefined);
    setGlobalHotkey.mockReset().mockResolvedValue(undefined);
  });

  it('defaults to Ctrl+Space', async () => {
    const { globalHotkey, DEFAULT_HOTKEY } = await fresh();
    expect(DEFAULT_HOTKEY).toBe('Ctrl+Space');
    expect(get(globalHotkey)).toBe('Ctrl+Space');
  });

  it('a set is mirrored to localStorage and survives a reload', async () => {
    let mod = await fresh();
    mod.globalHotkey.set('Ctrl+Alt+K');
    expect(get(mod.globalHotkey)).toBe('Ctrl+Alt+K');
    mod = await fresh();
    expect(get(mod.globalHotkey)).toBe('Ctrl+Alt+K');
  });

  it('applyGlobalHotkey hands the converted accelerator to the backend', async () => {
    const { globalHotkey, applyGlobalHotkey } = await fresh();
    globalHotkey.set('Ctrl+Alt+K');
    await applyGlobalHotkey();
    expect(setGlobalHotkey).toHaveBeenCalledWith('Control+Alt+K');
  });

  it('turning it off applies null', async () => {
    const { globalHotkey, applyGlobalHotkey } = await fresh();
    globalHotkey.set(null);
    await applyGlobalHotkey();
    expect(setGlobalHotkey).toHaveBeenCalledWith(null);
  });

  it('records a backend error, and clears it once a later apply succeeds', async () => {
    const { globalHotkey, globalHotkeyError, applyGlobalHotkey } = await fresh();
    setGlobalHotkey.mockRejectedValueOnce(new Error('Space is already in use'));
    globalHotkey.set('Ctrl+Space');
    await applyGlobalHotkey();
    expect(get(globalHotkeyError)).toBe('Space is already in use');
    await applyGlobalHotkey();
    expect(get(globalHotkeyError)).toBeNull();
  });

  it('hydrate applies the stored value without clobbering a fresh user change', async () => {
    backend.get.mockResolvedValue('Ctrl+Alt+K');
    const { globalHotkey } = await fresh();
    await globalHotkey.hydrate();
    expect(get(globalHotkey)).toBe('Ctrl+Alt+K');
    globalHotkey.set('Ctrl+Space'); // user acts before a later hydrate resolves
    backend.get.mockResolvedValue('Ctrl+Alt+K');
    await globalHotkey.hydrate();
    expect(get(globalHotkey)).toBe('Ctrl+Space');
  });
});
