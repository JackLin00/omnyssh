import { derived, get, writable, type Writable } from 'svelte/store';
import type { QuickGroupDto } from '$lib/bindings';
import { listQuickCommands, saveQuickCommands } from '$lib/ipc/commands';
import { commandBytes } from '$lib/screens/quickCommand';
import { lastError } from './notifications';

// The quick command groups (quick_commands.toml) behind the terminal and serial tabs'
// command bar, plus two UI prefs: the group the bar shows and whether it is collapsed.
// The prefs persist like the others (tauri-plugin-store + a localStorage mirror).
const STORE_FILE = 'settings.json';

function persisted<T>(localKey: string, storeKey: string, fallback: T, parse: (raw: string) => T) {
  let initial = fallback;
  try {
    const raw = localStorage.getItem(localKey);
    if (raw !== null) initial = parse(raw);
  } catch {
    // localStorage unavailable: start from the fallback.
  }
  const store = writable<T>(initial);
  let interacted = false;
  function apply(value: T, user: boolean): void {
    store.set(value);
    try {
      localStorage.setItem(localKey, JSON.stringify(value));
    } catch {
      // The plugin store copy is canonical.
    }
    if (user) {
      interacted = true;
      void (async () => {
        try {
          const { load } = await import('@tauri-apps/plugin-store');
          const s = await load(STORE_FILE);
          await s.set(storeKey, value);
          await s.save();
        } catch {
          // Not under Tauri: the mirror suffices.
        }
      })();
    }
  }
  return {
    subscribe: store.subscribe,
    set: (value: T) => apply(value, true),
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const s = await load(STORE_FILE);
        const saved = await s.get<T>(storeKey);
        if (!interacted && saved !== undefined && saved !== null) apply(saved, false);
      } catch {
        // Store unreachable: keep the mirrored value.
      }
    }
  };
}

const parseJson = <T>(fallback: T) => (raw: string): T => {
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
};

export const quickGroups: Writable<QuickGroupDto[]> = writable([]);
/** The group the bar shows, by name; empty means the first group. */
export const selectedGroup = persisted('omnyssh-quick-group', 'quickGroup', '', parseJson(''));
export const quickBarCollapsed = persisted(
  'omnyssh-quick-bar-collapsed',
  'quickBarCollapsed',
  false,
  parseJson(false)
);

export const currentGroup = derived([quickGroups, selectedGroup], ([groups, name]) =>
  groups.find((g) => g.name === name) ?? groups[0] ?? null
);

export async function loadQuickCommands(): Promise<void> {
  try {
    const groups = await listQuickCommands();
    quickGroups.set(Array.isArray(groups) ? groups : []);
  } catch (e) {
    lastError.set(e instanceof Error ? e.message : String(e));
  }
}

/** Save every group; the bar shows what was saved. Throws so a dialog can show why. */
export async function saveQuickGroups(groups: QuickGroupDto[]): Promise<void> {
  await saveQuickCommands(groups);
  quickGroups.set(groups);
}

/** The bytes of the current group's `n`th command (1-based), for Alt+N. */
export function quickCommandBytes(n: number): Uint8Array | null {
  const cmd = get(currentGroup)?.commands[n - 1];
  return cmd ? commandBytes(cmd, false) : null;
}
