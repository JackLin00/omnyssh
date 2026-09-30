// Customizable terminal shortcuts (Settings → Keyboard shortcuts). A chord is a string
// like "Ctrl+Shift+C" or "Alt+Shift+Equal": modifiers in a fixed order, then a key token.
// A letter token is the ASCII letter the layout types, so Dvorak's C is the key labelled
// C; where the layout types no ASCII letter (Cyrillic, or macOS Option composing ∑) the
// physical key's letter stands in. Any other token is a KeyboardEvent.code, a physical
// key. Matching turns the event into the same string and compares.
import type { KeyPress } from './terminalInput';

export type TerminalAction = 'copy' | 'paste' | 'splitRight' | 'splitDown' | 'closePane';
export type Chord = string;
/** The chord bound to each action; null leaves the keys to the shell. */
export type Bindings = Record<TerminalAction, Chord | null>;

/** Every action, in the order Settings lists them. */
export const TERMINAL_ACTIONS: readonly { action: TerminalAction; label: string }[] = [
  { action: 'copy', label: 'Copy' },
  { action: 'paste', label: 'Paste' },
  { action: 'splitRight', label: 'Split right' },
  { action: 'splitDown', label: 'Split down' },
  { action: 'closePane', label: 'Close pane' }
];

/** Windows Terminal's chords. macOS copies and pastes with Cmd+C / Cmd+V through the
 *  Edit menu, so those two start unbound there. */
export function defaultBindings(mac: boolean): Bindings {
  return {
    copy: mac ? null : 'Ctrl+Shift+C',
    paste: mac ? null : 'Ctrl+Shift+V',
    splitRight: 'Alt+Shift+Equal',
    splitDown: 'Alt+Shift+Minus',
    closePane: 'Ctrl+Shift+W'
  };
}

export interface ParsedChord {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  key: string;
}

const MODIFIERS = ['Ctrl', 'Alt', 'Shift', 'Meta'] as const;

function toChord(p: ParsedChord): Chord {
  const mods = [p.ctrl && 'Ctrl', p.alt && 'Alt', p.shift && 'Shift', p.meta && 'Meta'];
  return [...mods.filter(Boolean), p.key].join('+');
}

export function parseChord(chord: Chord): ParsedChord | null {
  const parts = chord.split('+');
  const key = parts.pop();
  if (!key || parts.some((p) => !(MODIFIERS as readonly string[]).includes(p))) return null;
  return {
    ctrl: parts.includes('Ctrl'),
    alt: parts.includes('Alt'),
    shift: parts.includes('Shift'),
    meta: parts.includes('Meta'),
    key
  };
}

const MODIFIER_KEYS = new Set([
  'Control',
  'Shift',
  'Alt',
  'AltGraph',
  'Meta',
  'CapsLock',
  // Some webviews name the Windows/Super/Cmd key 'OS' rather than 'Meta'.
  'OS',
  'Super',
  'Hyper',
  'Fn',
  'NumLock',
  'ScrollLock'
]);
// Defensive fallback for a webview that reports none of the names above but still gives
// the physical modifier key's own code (e.g. an unnamed key on 'AltRight').
const MODIFIER_CODE = /^(Control|Shift|Alt|Meta|OS)(Left|Right)$/;

/** The chord a key event spells, or null for a lone modifier or an IME keystroke
 *  (keyCode 229 marks the keydown that starts a composition). */
export function chordFromEvent(e: KeyPress): Chord | null {
  if (e.isComposing || e.keyCode === 229 || MODIFIER_KEYS.has(e.key) || MODIFIER_CODE.test(e.code)) {
    return null;
  }
  const letter = /^[a-z]$/i.test(e.key) ? e.key.toUpperCase() : /^Key([A-Z])$/.exec(e.code)?.[1];
  const key = letter ?? e.code;
  if (!key) return null;
  return toChord({ ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey, meta: e.metaKey, key });
}

/** The action `e` triggers under `bindings`. Every event type matches, so no half of a
 *  chord reaches the shell; the caller acts on keydown only. */
export function matchTerminalAction(e: KeyPress, bindings: Bindings): TerminalAction | null {
  const pressed = chordFromEvent(e);
  if (!pressed) return null;
  return TERMINAL_ACTIONS.find(({ action }) => bindings[action] === pressed)?.action ?? null;
}

const KEY_LABELS: Record<string, string> = {
  Equal: '=',
  Minus: '-',
  BracketLeft: '[',
  BracketRight: ']',
  Backslash: '\\',
  Semicolon: ';',
  Quote: "'",
  Comma: ',',
  Period: '.',
  Slash: '/',
  Backquote: '`',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→'
};

/** A chord as the keyboard prints it, with the platform's names for Alt and Meta. */
export function formatChord(chord: Chord, mac: boolean): string {
  const p = parseChord(chord);
  if (!p) return chord;
  const key = KEY_LABELS[p.key] ?? /^Digit(\d)$/.exec(p.key)?.[1] ?? p.key;
  const mods = [
    p.ctrl && 'Ctrl',
    p.alt && (mac ? 'Option' : 'Alt'),
    p.shift && 'Shift',
    p.meta && (mac ? 'Cmd' : 'Win')
  ];
  return [...mods.filter(Boolean), key].join('+');
}

export type Validation = { ok: true; warning: string | null } | { ok: false; error: string };

/** Whether `chord` may be bound to `action` given the current `bindings`. */
export function validateChord(action: TerminalAction, chord: Chord, bindings: Bindings): Validation {
  const p = parseChord(chord);
  if (!p) return { ok: false, error: 'Not a valid shortcut' };
  if (!p.ctrl && !p.alt && !p.meta) {
    return { ok: false, error: 'Use Ctrl, Alt or Cmd in a shortcut, so it never takes a key you type' };
  }
  const other = TERMINAL_ACTIONS.find((a) => a.action !== action && bindings[a.action] === chord);
  if (other) return { ok: false, error: `Already used by “${other.label}”` };
  const shellControl = p.ctrl && !p.alt && !p.shift && !p.meta && /^[A-Z]$/.test(p.key);
  // WebView2 reports AltGr (used on many non-US layouts to type @, {, [, …) as Ctrl+Alt;
  // shellControl can never also be true here, since that one requires no Alt.
  const altGr = p.ctrl && p.alt && !p.shift && !p.meta;
  let warning: string | null = null;
  if (shellControl) {
    warning = `Ctrl+${p.key} is the shell's ^${p.key}; the terminal will no longer send it`;
  } else if (altGr) {
    warning = 'Ctrl+Alt is AltGr on many keyboards; this may stop a character like @ or { from being typed';
  }
  return { ok: true, warning };
}
