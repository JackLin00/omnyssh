import { get, writable } from 'svelte/store';
import { parseChord, type Chord } from '$lib/screens/terminalShortcuts';
import { setGlobalHotkey } from '$lib/ipc/commands';

// The system-wide show/hide hotkey (plan J), Ctrl+Space by default as in Tabby. A UI
// pref persisted like the other terminal chords (tauri-plugin-store + a localStorage
// mirror for first paint), but it can also be off (`null`) rather than merely unbound.

/** Code names `tauri-plugin-global-shortcut`'s parser (global-hotkey's `parse_key`)
 *  recognizes beyond single letters, digits and function keys, which it takes by name
 *  directly. A chord recorder can still capture a code with no entry here (e.g.
 *  "ContextMenu") — those can't become an accelerator. */
const SUPPORTED_CODES = new Set([
  'Backquote',
  'Backslash',
  'BracketLeft',
  'BracketRight',
  'Pause',
  'Comma',
  'Equal',
  'Minus',
  'Period',
  'Quote',
  'Semicolon',
  'Slash',
  'Backspace',
  'CapsLock',
  'Enter',
  'Space',
  'Tab',
  'Delete',
  'End',
  'Home',
  'Insert',
  'PageDown',
  'PageUp',
  'PrintScreen',
  'ScrollLock',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'ArrowUp',
  'NumLock',
  'NumpadAdd',
  'NumpadDecimal',
  'NumpadDivide',
  'NumpadEnter',
  'NumpadEqual',
  'NumpadMultiply',
  'NumpadSubtract',
  'Escape',
  'AudioVolumeDown',
  'AudioVolumeUp',
  'AudioVolumeMute',
  'MediaPlay',
  'MediaPause',
  'MediaPlayPause',
  'MediaStop',
  'MediaTrackNext',
  'MediaTrackPrevious'
]);

function isSupportedKey(key: string): boolean {
  return (
    /^[A-Z]$/.test(key) ||
    /^Digit[0-9]$/.test(key) ||
    /^Numpad[0-9]$/.test(key) ||
    /^F([1-9]|1[0-9]|2[0-4])$/.test(key) ||
    SUPPORTED_CODES.has(key)
  );
}

/** The chord as the plugin's accelerator syntax: `Ctrl`/`Alt`/`Shift` pass straight
 *  through (its parser takes them case-insensitively), `Meta` becomes `Super` — the
 *  only modifier name it does not also know as `Meta`. Null when `chord` cannot
 *  parse, or names a key the parser has no entry for. */
export function chordToAccelerator(chord: Chord): string | null {
  const p = parseChord(chord);
  if (!p || !isSupportedKey(p.key)) return null;
  const mods = [p.ctrl && 'Control', p.alt && 'Alt', p.shift && 'Shift', p.meta && 'Super'];
  return [...mods.filter((m): m is string => Boolean(m)), p.key].join('+');
}

const LOCAL_KEY = 'omnyssh-global-hotkey';
const STORE_FILE = 'settings.json';
const STORE_KEY = 'globalHotkey';
export const DEFAULT_HOTKEY: Chord = 'Ctrl+Space';

/** A stored value leniently: null is off, a readable chord is kept, anything else
 *  (corrupted storage) falls back to the default. */
function sanitize(value: unknown): Chord | null {
  if (value === null) return null;
  return typeof value === 'string' && parseChord(value) ? value : DEFAULT_HOTKEY;
}

function mirrored(): Chord | null {
  try {
    const raw = localStorage.getItem(LOCAL_KEY);
    return raw === null ? DEFAULT_HOTKEY : sanitize(JSON.parse(raw));
  } catch {
    return DEFAULT_HOTKEY; // localStorage unavailable, or held something unreadable
  }
}

function mirrorLocal(value: Chord | null): void {
  try {
    localStorage.setItem(LOCAL_KEY, JSON.stringify(value));
  } catch {
    // localStorage unavailable (hardened webview): the store copy is canonical.
  }
}

async function persistStore(value: Chord | null): Promise<void> {
  try {
    const { load } = await import('@tauri-apps/plugin-store');
    const store = await load(STORE_FILE);
    await store.set(STORE_KEY, value);
    await store.save();
  } catch {
    // Not under Tauri (tests, vite preview): the localStorage mirror suffices.
  }
}

function createGlobalHotkey() {
  const initial = mirrored();
  const { subscribe, set: setStore } = writable<Chord | null>(initial);
  let interacted = false;

  function apply(value: Chord | null, user: boolean): void {
    setStore(value);
    mirrorLocal(value);
    if (user) {
      interacted = true;
      void persistStore(value);
    }
  }

  return {
    subscribe,
    set: (value: Chord | null) => apply(value, true),
    /** Back to the default hotkey. */
    reset: () => apply(DEFAULT_HOTKEY, true),
    /** Reconcile with the canonical tauri-plugin-store value once Tauri is reachable. */
    async hydrate(): Promise<void> {
      try {
        const { load } = await import('@tauri-apps/plugin-store');
        const store = await load(STORE_FILE);
        const saved = await store.get<unknown>(STORE_KEY);
        if (!interacted && saved !== undefined) apply(sanitize(saved), false);
      } catch {
        // Store unreachable: keep the mirrored value.
      }
    }
  };
}

export const globalHotkey = createGlobalHotkey();

/** Why the backend rejected the current hotkey (another program holds it, or the OS
 *  format is unsupported), or null when it is registered as asked. */
export const globalHotkeyError = writable<string | null>(null);

/** Convert the current value and hand it to the backend; null turns the hotkey off.
 *  Clears `globalHotkeyError` on success, sets it on failure. */
export async function applyGlobalHotkey(): Promise<void> {
  const value = get(globalHotkey);
  try {
    if (value === null) {
      await setGlobalHotkey(null);
    } else {
      const accelerator = chordToAccelerator(value);
      if (accelerator === null) throw new Error('Not a valid shortcut');
      await setGlobalHotkey(accelerator);
    }
    globalHotkeyError.set(null);
  } catch (e) {
    globalHotkeyError.set(e instanceof Error ? e.message : String(e));
  }
}
