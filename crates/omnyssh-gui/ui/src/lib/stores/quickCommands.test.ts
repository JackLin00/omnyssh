// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

const list = vi.fn();
const save = vi.fn();
vi.mock('$lib/ipc/commands', () => ({ listQuickCommands: list, saveQuickCommands: save }));
vi.mock('@tauri-apps/plugin-store', () => ({
  load: vi.fn(async () => {
    throw new Error('no tauri');
  })
}));

async function fresh() {
  vi.resetModules();
  return await import('./quickCommands');
}

const groups = [
  { name: 'A', commands: [{ label: 'ls', kind: 'text' as const, payload: 'ls', ending: 'cr' as const }] },
  { name: 'B', commands: [] }
];

describe('quick commands store', () => {
  beforeEach(() => {
    localStorage.clear();
    list.mockReset().mockResolvedValue(groups);
    save.mockReset().mockResolvedValue(undefined);
  });

  it('loads the groups and picks the first when none is chosen', async () => {
    const s = await fresh();
    await s.loadQuickCommands();
    expect(get(s.quickGroups)).toEqual(groups);
    expect(get(s.currentGroup)?.name).toBe('A');
  });

  it('keeps a chosen group across a reload, and falls back when it is gone', async () => {
    let s = await fresh();
    await s.loadQuickCommands();
    s.selectedGroup.set('B');
    s = await fresh();
    await s.loadQuickCommands();
    expect(get(s.currentGroup)?.name).toBe('B');
    list.mockResolvedValue([groups[0]]);
    await s.loadQuickCommands();
    expect(get(s.currentGroup)?.name).toBe('A');
  });

  it('treats a non-list from the backend as no groups', async () => {
    list.mockResolvedValue(null);
    const s = await fresh();
    await s.loadQuickCommands();
    expect(get(s.quickGroups)).toEqual([]);
  });

  it('saves through the backend and then shows the saved groups', async () => {
    const s = await fresh();
    await s.saveQuickGroups([groups[1]]);
    expect(save).toHaveBeenCalledWith([groups[1]]);
    expect(get(s.quickGroups)).toEqual([groups[1]]);
  });

  it('gives the bytes of the current group\'s Nth command', async () => {
    const s = await fresh();
    await s.loadQuickCommands();
    // Compared as plain arrays: under jsdom, `TextEncoder` and a literal `new
    // Uint8Array(...)` construct in different realms, so `toEqual` on the typed
    // arrays themselves can fail despite identical bytes.
    expect(Array.from(s.quickCommandBytes(1)!)).toEqual([0x6c, 0x73, 0x0d]);
    expect(s.quickCommandBytes(2)).toBeNull();
  });

  it('remembers whether the bar is collapsed', async () => {
    let s = await fresh();
    s.quickBarCollapsed.set(true);
    s = await fresh();
    expect(get(s.quickBarCollapsed)).toBe(true);
  });

  it('sets quickLoadError on a failed load and clears it on the next successful one', async () => {
    list.mockRejectedValueOnce(new Error('Failed to parse quick_commands.toml'));
    const s = await fresh();
    await s.loadQuickCommands();
    expect(get(s.quickLoadError)).toBe('Failed to parse quick_commands.toml');

    await s.loadQuickCommands();
    expect(get(s.quickLoadError)).toBeNull();
  });
});
