import { writable } from 'svelte/store';

// Mouse clipboard in the terminals, PuTTY-style: copy a selection as soon as the drag
// ends, paste on right-click. Both on by default and switchable in Settings. They
// persist like the other UI-chrome prefs (tauri-plugin-store + a localStorage mirror
// for first paint), in the same shape as `streamerMode`.
const STORE_FILE = 'settings.json';

function persistedFlag(localKey: string, storeKey: string, fallback: boolean) {
  function mirrored(): boolean {
    try {
      const raw = localStorage.getItem(localKey);
      return raw == null ? fallback : raw === 'true';
    } catch {
      return fallback; // localStorage unavailable
    }
  }

  function mirrorLocal(on: boolean): void {
    try {
      localStorage.setItem(localKey, String(on));
    } catch {
      // localStorage unavailable (hardened webview): the store copy is canonical.
    }
  }

  async function persistStore(on: boolean): Promise<void> {
    try {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(STORE_FILE);
      await store.set(storeKey, on);
      await store.save();
    } catch {
      // Not under Tauri (tests, vite preview): the localStorage mirror suffices.
    }
  }

  const initial = mirrored();
  const { subscribe, set: setStore } = writable<boolean>(initial);
  let current = initial;
  let interacted = false;

  function apply(on: boolean, user: boolean): void {
    current = on;
    setStore(on);
    mirrorLocal(on);
    if (user) {
      interacted = true;
      void persistStore(on);
    }
  }

  return {
    subscribe,
    set: (on: boolean) => apply(on, true),
    toggle: () => apply(!current, true),
    /** Reconcile with the canonical tauri-plugin-store value once Tauri is reachable. */
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const store = await load(STORE_FILE);
        const saved = await store.get<boolean>(storeKey);
        if (!interacted && typeof saved === 'boolean') apply(saved, false);
      } catch {
        // Store unreachable: keep the mirrored value.
      }
    }
  };
}

/** Copy the terminal selection to the clipboard when a mouse drag ends. */
export const copyOnSelect = persistedFlag('omnyssh-copy-on-select', 'copyOnSelect', true);
/** Right-click in a terminal pastes the clipboard instead of opening the context menu. */
export const rightClickPaste = persistedFlag('omnyssh-right-click-paste', 'rightClickPaste', true);
/** Stamp a serial monitor's lines with their arrival time. */
export const serialTimestamps = persistedFlag('omnyssh-serial-timestamps', 'serialTimestamps', false);

function persistedNumber(
  localKey: string,
  storeKey: string,
  fallback: number,
  min: number,
  max: number
) {
  function clamp(n: number): number {
    return Number.isFinite(n) ? Math.min(max, Math.max(min, Math.round(n))) : fallback;
  }

  function mirrored(): number {
    try {
      const raw = localStorage.getItem(localKey);
      return raw == null ? fallback : clamp(Number(raw));
    } catch {
      return fallback; // localStorage unavailable
    }
  }

  function mirrorLocal(n: number): void {
    try {
      localStorage.setItem(localKey, String(n));
    } catch {
      // localStorage unavailable (hardened webview): the store copy is canonical.
    }
  }

  async function persistStore(n: number): Promise<void> {
    try {
      const { load } = await import('@tauri-apps/plugin-store');
      const store = await load(STORE_FILE);
      await store.set(storeKey, n);
      await store.save();
    } catch {
      // Not under Tauri (tests, vite preview): the localStorage mirror suffices.
    }
  }

  const initial = mirrored();
  const { subscribe, set: setStore } = writable<number>(initial);
  let current = initial;
  let interacted = false;

  function apply(n: number, user: boolean): void {
    current = n;
    setStore(n);
    mirrorLocal(n);
    if (user) {
      interacted = true;
      void persistStore(n);
    }
  }

  return {
    subscribe,
    set: (n: number) => apply(clamp(n), true),
    step: (delta: number) => apply(clamp(current + delta), true),
    reset: () => apply(fallback, true),
    /** Reconcile with the canonical tauri-plugin-store value once Tauri is reachable. */
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const store = await load(STORE_FILE);
        const saved = await store.get<number>(storeKey);
        if (!interacted && typeof saved === 'number') apply(clamp(saved), false);
      } catch {
        // Store unreachable: keep the mirrored value.
      }
    }
  };
}

export const FONT_SIZE_MIN = 8;
export const FONT_SIZE_MAX = 32;
export const FONT_SIZE_DEFAULT = 13;
/** The terminals' font size in px, shared by every SSH and serial tab. */
export const terminalFontSize = persistedNumber(
  'omnyssh-terminal-font-size',
  'terminalFontSize',
  FONT_SIZE_DEFAULT,
  FONT_SIZE_MIN,
  FONT_SIZE_MAX
);
