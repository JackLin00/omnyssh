import { derived, writable } from 'svelte/store';
import { resolveScheme, TERMINAL_SCHEMES } from '$lib/theme/terminalSchemes';
import { theme } from './theme';

// The chosen terminal colour scheme persists like the other UI prefs (tauri-plugin-store
// + a localStorage mirror), in the same shape as `terminalPrefs.ts`. `sanitize` keeps a
// stale or corrupted stored value from wedging the picker on an id that no longer exists.
const LOCAL_KEY = 'omnyssh-terminal-scheme';
const STORE_FILE = 'settings.json';
const STORE_KEY = 'terminalScheme';
const DEFAULT_ID = 'omnyssh';

function sanitize(id: unknown): string {
  return typeof id === 'string' && TERMINAL_SCHEMES.some((s) => s.id === id) ? id : DEFAULT_ID;
}

function mirrored(): string {
  try {
    return sanitize(localStorage.getItem(LOCAL_KEY));
  } catch {
    return DEFAULT_ID; // localStorage unavailable
  }
}

function createTerminalScheme() {
  const { subscribe, set: setStore } = writable<string>(mirrored());
  let interacted = false;

  function mirrorLocal(id: string): void {
    try {
      localStorage.setItem(LOCAL_KEY, id);
    } catch {
      // localStorage unavailable (hardened webview): the store copy is canonical.
    }
  }

  async function persistStore(id: string): Promise<void> {
    try {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(STORE_FILE);
      await store.set(STORE_KEY, id);
      await store.save();
    } catch {
      // Not under Tauri (tests, vite preview): the localStorage mirror suffices.
    }
  }

  function apply(id: string, user: boolean): void {
    const safe = sanitize(id);
    setStore(safe);
    mirrorLocal(safe);
    if (user) {
      interacted = true;
      void persistStore(safe);
    }
  }

  return {
    subscribe,
    set: (id: string) => apply(id, true),
    /** Reconcile with the canonical tauri-plugin-store value once Tauri is reachable. */
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const store = await load(STORE_FILE);
        const saved = await store.get<string>(STORE_KEY);
        if (!interacted && typeof saved === 'string') apply(saved, false);
      } catch {
        // Store unreachable: keep the mirrored value.
      }
    }
  };
}

export const terminalScheme = createTerminalScheme();

/** The xterm theme every terminal shows: the chosen scheme in the app theme's variant. */
export const terminalColors = derived([theme, terminalScheme], ([t, id]) => resolveScheme(id, t));
