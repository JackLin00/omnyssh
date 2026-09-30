// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

const backend = { get: vi.fn(), set: vi.fn(), save: vi.fn() };
vi.mock('@tauri-apps/plugin-store', () => ({ load: vi.fn(async () => backend) }));

async function fresh() {
  vi.resetModules();
  return (await import('./terminalShortcuts')).terminalShortcuts;
}

describe('terminal shortcuts store', () => {
  beforeEach(() => {
    localStorage.clear();
    backend.get.mockReset();
    backend.set.mockReset().mockResolvedValue(undefined);
    backend.save.mockReset().mockResolvedValue(undefined);
  });

  it('starts at the defaults', async () => {
    const shortcuts = await fresh();
    expect(get(shortcuts)).toMatchObject({ copy: 'Ctrl+Shift+C', closePane: 'Ctrl+Shift+W' });
  });

  it('a change or a clear sticks across a reload and persists only the overrides', async () => {
    let shortcuts = await fresh();
    shortcuts.set('closePane', 'Ctrl+Shift+Q');
    shortcuts.set('paste', null);
    await vi.waitFor(() =>
      expect(backend.set).toHaveBeenLastCalledWith('terminalShortcuts', {
        closePane: 'Ctrl+Shift+Q',
        paste: null
      })
    );
    shortcuts = await fresh();
    expect(get(shortcuts)).toMatchObject({ closePane: 'Ctrl+Shift+Q', paste: null, copy: 'Ctrl+Shift+C' });
  });

  it('reset returns one action to its default, resetAll every action', async () => {
    const shortcuts = await fresh();
    shortcuts.set('closePane', 'Ctrl+Shift+Q');
    shortcuts.set('copy', null);
    shortcuts.reset('closePane');
    expect(get(shortcuts).closePane).toBe('Ctrl+Shift+W');
    expect(get(shortcuts).copy).toBeNull();
    shortcuts.resetAll();
    expect(get(shortcuts).copy).toBe('Ctrl+Shift+C');
  });

  it('drops unknown actions and unreadable chords from storage', async () => {
    localStorage.setItem(
      'omnyssh-terminal-shortcuts',
      JSON.stringify({ closePane: 'Hyper+Q', bogus: 'Ctrl+X', copy: 42, splitDown: 'Alt+Minus' })
    );
    const shortcuts = await fresh();
    expect(get(shortcuts)).toMatchObject({
      closePane: 'Ctrl+Shift+W',
      copy: 'Ctrl+Shift+C',
      splitDown: 'Alt+Minus'
    });
    expect(get(shortcuts)).not.toHaveProperty('bogus');
  });

  it('hydrate applies the stored overrides without clobbering a fresh user change', async () => {
    backend.get.mockResolvedValue({ splitRight: 'Alt+Shift+R' });
    let shortcuts = await fresh();
    await shortcuts.hydrate();
    expect(get(shortcuts).splitRight).toBe('Alt+Shift+R');

    shortcuts = await fresh();
    shortcuts.set('copy', null); // user acts before hydrate runs
    await shortcuts.hydrate();
    expect(get(shortcuts).copy).toBeNull();
  });
});
