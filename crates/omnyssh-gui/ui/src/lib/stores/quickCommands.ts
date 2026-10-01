import { derived, get, writable, type Writable } from 'svelte/store';
import type { QuickGroupDto } from '$lib/bindings';
import { listQuickCommands, saveQuickCommands } from '$lib/ipc/commands';
import { commandBytes } from '$lib/screens/quickCommand';

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
  // Two `set` calls in the same tick must reach the backend in order, like
  // `terminalShortcuts`'s store does: a bare `void persistStore(next)` per call races
  // the dynamic `import()` and can let an earlier write land after a later one.
  let persistChain: Promise<void> = Promise.resolve();
  function schedulePersist(value: T): void {
    persistChain = persistChain.then(async () => {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const s = await load(STORE_FILE);
        await s.set(storeKey, value);
        await s.save();
      } catch {
        // Not under Tauri: the mirror suffices.
      }
    });
  }
  function apply(value: T, user: boolean): void {
    store.set(value);
    try {
      localStorage.setItem(localKey, JSON.stringify(value));
    } catch {
      // The plugin store copy is canonical.
    }
    if (user) {
      interacted = true;
      schedulePersist(value);
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
        // A shape that doesn't match the fallback (a stale or corrupted store write)
        // is not applied — the mirrored value stays rather than wedging the UI prefs.
        if (!interacted && saved !== undefined && saved !== null && typeof saved === typeof fallback) {
          apply(saved, false);
        }
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

/** Set when the last load failed (a malformed `quick_commands.toml`), cleared on the
 *  next successful one. The bar reads this to refuse to offer editing — saving over a
 *  file it couldn't parse would lose whatever is actually on disk. */
export const quickLoadError: Writable<string | null> = writable(null);

export async function loadQuickCommands(): Promise<void> {
  try {
    const groups = await listQuickCommands();
    quickGroups.set(Array.isArray(groups) ? groups : []);
    quickLoadError.set(null);
  } catch (e) {
    quickLoadError.set(e instanceof Error ? e.message : String(e));
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
