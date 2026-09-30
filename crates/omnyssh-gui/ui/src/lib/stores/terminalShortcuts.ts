import { derived, writable } from 'svelte/store';
import { isMac } from '$lib/platform';
import {
  defaultBindings,
  parseChord,
  TERMINAL_ACTIONS,
  type Bindings,
  type Chord,
  type TerminalAction
} from '$lib/screens/terminalShortcuts';

// The terminal chords the user rebound in Settings → Keyboard shortcuts. Only the
// overrides are kept, so a default that changes in a later release still reaches
// everyone who never touched it. Persisted like the other UI prefs (tauri-plugin-store
// + a localStorage mirror for first paint).
const LOCAL_KEY = 'omnyssh-terminal-shortcuts';
const STORE_FILE = 'settings.json';
const STORE_KEY = 'terminalShortcuts';

type Overrides = Partial<Record<TerminalAction, Chord | null>>;

/** Keep known actions bound to a readable chord or to nothing; drop the rest. */
function sanitize(raw: unknown): Overrides {
  const out: Overrides = {};
  if (typeof raw !== 'object' || raw === null) return out;
  for (const { action } of TERMINAL_ACTIONS) {
    const value = (raw as Record<string, unknown>)[action];
    if (value === null) out[action] = null;
    else if (typeof value === 'string' && parseChord(value)) out[action] = value;
  }
  return out;
}

function mirrored(): Overrides {
  try {
    return sanitize(JSON.parse(localStorage.getItem(LOCAL_KEY) ?? '{}'));
  } catch {
    return {};
  }
}

function mirrorLocal(overrides: Overrides): void {
  try {
    localStorage.setItem(LOCAL_KEY, JSON.stringify(overrides));
  } catch {
    // localStorage unavailable (hardened webview): the store copy is canonical.
  }
}

async function persistStore(overrides: Overrides): Promise<void> {
  try {
    const { load } = await import('@tauri-apps/plugin-store');
    const store = await load(STORE_FILE);
    await store.set(STORE_KEY, overrides);
    await store.save();
  } catch {
    // Not under Tauri (tests, vite preview): the localStorage mirror suffices.
  }
}

function createTerminalShortcuts() {
  const defaults = defaultBindings(isMac);
  const initial = mirrored();
  const overrides = writable<Overrides>(initial);
  let current = initial;
  let interacted = false;
  const bindings = derived(overrides, (o): Bindings => ({ ...defaults, ...o }));

  // Two `set` calls in the same tick must reach the backend in order — a bare
  // `void persistStore(next)` per call races the dynamic `import()` (the first call can
  // resolve after the second, dropping the earlier write's fields from the last save).
  let persistChain: Promise<void> = Promise.resolve();
  function schedulePersist(next: Overrides): void {
    persistChain = persistChain.then(() => persistStore(next));
  }

  function apply(next: Overrides, user: boolean): void {
    current = next;
    overrides.set(next);
    mirrorLocal(next);
    if (user) {
      interacted = true;
      schedulePersist(next);
    }
  }

  /** Drop `action`'s override, if any, from `overrides`. */
  function withoutOverride(overrides: Overrides, action: TerminalAction): Overrides {
    const { [action]: _dropped, ...rest } = overrides;
    return rest;
  }

  return {
    subscribe: bindings.subscribe,
    /** Bind `action` to `chord`, or to nothing with null. Setting it back to its default
     *  drops the override rather than storing a redundant one, so a default that changes
     *  in a later release still reaches this user. */
    set: (action: TerminalAction, chord: Chord | null) =>
      apply(
        chord === defaults[action]
          ? withoutOverride(current, action)
          : { ...current, [action]: chord },
        true
      ),
    /** Back to the default for `action`. */
    reset: (action: TerminalAction) => apply(withoutOverride(current, action), true),
    resetAll: () => apply({}, true),
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

export const terminalShortcuts = createTerminalShortcuts();
