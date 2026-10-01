// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';
import { resolveScheme } from '$lib/theme/terminalSchemes';

// The chosen terminal colour scheme persists like the other UI prefs (tauri-plugin-store
// + a localStorage mirror), following terminalPrefs.ts's pattern, but sanitizes against
// the known scheme ids so a stale/bogus stored value can't wedge the picker.
const backend = { get: vi.fn(), set: vi.fn(), save: vi.fn() };
vi.mock('@tauri-apps/plugin-store', () => ({ load: vi.fn(async () => backend) }));

async function fresh() {
  vi.resetModules();
  return await import('./terminalScheme');
}

describe('terminal scheme preference', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.setAttribute('data-theme', 'dark');
    backend.get.mockReset();
    backend.set.mockReset().mockResolvedValue(undefined);
    backend.save.mockReset().mockResolvedValue(undefined);
  });

  it('defaults to the app\'s own scheme', async () => {
    const { terminalScheme } = await fresh();
    expect(get(terminalScheme)).toBe('omnyssh');
  });

  it('a pick is mirrored to localStorage and survives a reload', async () => {
    let { terminalScheme } = await fresh();
    terminalScheme.set('catppuccin');
    expect(get(terminalScheme)).toBe('catppuccin');
    expect(localStorage.getItem('omnyssh-terminal-scheme')).toBe('catppuccin');
    ({ terminalScheme } = await fresh());
    expect(get(terminalScheme)).toBe('catppuccin');
  });

  it('falls back to the app\'s own scheme for an unknown id, including one read from storage', async () => {
    let { terminalScheme } = await fresh();
    terminalScheme.set('not-a-scheme');
    expect(get(terminalScheme)).toBe('omnyssh');

    localStorage.setItem('omnyssh-terminal-scheme', 'also-bogus');
    ({ terminalScheme } = await fresh());
    expect(get(terminalScheme)).toBe('omnyssh');
  });

  it('writes the canonical tauri-plugin-store on a user pick', async () => {
    const { terminalScheme } = await fresh();
    terminalScheme.set('nord');
    await vi.waitFor(() => {
      expect(backend.set).toHaveBeenCalledWith('terminalScheme', 'nord');
      expect(backend.save).toHaveBeenCalled();
    });
  });

  it('hydrate applies the stored value without clobbering a fresh user pick', async () => {
    backend.get.mockResolvedValue('dracula');
    const { terminalScheme } = await fresh();
    terminalScheme.set('nord'); // user acts before the async hydrate resolves
    await terminalScheme.hydrate();
    expect(get(terminalScheme)).toBe('nord');
  });

  it('terminalColors follows the chosen scheme and the app theme', async () => {
    const { terminalScheme, terminalColors } = await fresh();
    const { theme } = await import('./theme');
    terminalScheme.set('catppuccin');
    theme.set('dark');
    expect(get(terminalColors)).toEqual(resolveScheme('catppuccin', 'dark'));
    theme.set('light');
    expect(get(terminalColors)).toEqual(resolveScheme('catppuccin', 'light'));
  });
});
