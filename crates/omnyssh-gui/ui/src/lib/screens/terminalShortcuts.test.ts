import { describe, expect, it } from 'vitest';
import type { KeyPress } from './terminalInput';
import {
  chordFromEvent,
  defaultBindings,
  formatChord,
  matchTerminalAction,
  parseChord,
  validateChord,
  type Bindings
} from './terminalShortcuts';

const press = (over: Partial<KeyPress>): KeyPress => ({
  type: 'keydown',
  key: 'C',
  code: 'KeyC',
  keyCode: 67,
  ctrlKey: true,
  shiftKey: true,
  altKey: false,
  metaKey: false,
  isComposing: false,
  ...over
});

const defaults: Bindings = defaultBindings(false);

describe('chordFromEvent', () => {
  it('names a letter chord by the letter the layout types', () => {
    expect(chordFromEvent(press({}))).toBe('Ctrl+Shift+C');
    expect(chordFromEvent(press({ key: 'c' }))).toBe('Ctrl+Shift+C');
    // Dvorak: the key labelled C sits on the physical I.
    expect(chordFromEvent(press({ key: 'C', code: 'KeyI' }))).toBe('Ctrl+Shift+C');
    expect(chordFromEvent(press({ key: 'J', code: 'KeyC' }))).toBe('Ctrl+Shift+J');
  });

  it('falls back to the physical letter when the layout types no ASCII letter', () => {
    // Russian layout on WebKitGTK: Cyrillic key, keyCode 0.
    expect(chordFromEvent(press({ key: 'С', keyCode: 0 }))).toBe('Ctrl+Shift+C');
    // macOS Option composes a character on the W key.
    expect(chordFromEvent(press({ ctrlKey: false, altKey: true, key: '∑', code: 'KeyW' }))).toBe(
      'Alt+Shift+W'
    );
  });

  it('names any other key by its physical code, modifiers in a fixed order', () => {
    expect(chordFromEvent(press({ ctrlKey: false, altKey: true, key: '+', code: 'Equal' }))).toBe(
      'Alt+Shift+Equal'
    );
    expect(
      chordFromEvent(press({ metaKey: true, altKey: true, key: '!', code: 'Digit1' }))
    ).toBe('Ctrl+Alt+Shift+Meta+Digit1');
  });

  it('ignores a lone modifier and composition', () => {
    expect(chordFromEvent(press({ key: 'Shift', code: 'ShiftLeft' }))).toBeNull();
    expect(chordFromEvent(press({ key: 'Control', code: 'ControlLeft' }))).toBeNull();
    expect(chordFromEvent(press({ isComposing: true }))).toBeNull();
    expect(chordFromEvent(press({ key: 'Process', keyCode: 229 }))).toBeNull();
  });
});

describe('matchTerminalAction', () => {
  it('finds each default chord', () => {
    expect(matchTerminalAction(press({}), defaults)).toBe('copy');
    expect(matchTerminalAction(press({ key: 'V', code: 'KeyV' }), defaults)).toBe('paste');
    expect(matchTerminalAction(press({ key: 'W', code: 'KeyW' }), defaults)).toBe('closePane');
    const alt = { ctrlKey: false, altKey: true };
    expect(matchTerminalAction(press({ ...alt, key: '+', code: 'Equal' }), defaults)).toBe('splitRight');
    expect(matchTerminalAction(press({ ...alt, key: '_', code: 'Minus' }), defaults)).toBe('splitDown');
  });

  it('matches every event type, so no half of a chord reaches the shell', () => {
    expect(matchTerminalAction(press({ type: 'keyup' }), defaults)).toBe('copy');
  });

  it('leaves bare Ctrl+C, extra modifiers and unbound chords alone', () => {
    expect(matchTerminalAction(press({ shiftKey: false, key: 'c' }), defaults)).toBeNull();
    expect(matchTerminalAction(press({ altKey: true }), defaults)).toBeNull();
    expect(matchTerminalAction(press({ key: 'X', code: 'KeyX' }), defaults)).toBeNull();
    expect(matchTerminalAction(press({ isComposing: true }), defaults)).toBeNull();
  });

  it('follows the bindings it is given, and a cleared action matches nothing', () => {
    const custom: Bindings = { ...defaults, copy: null, closePane: 'Ctrl+Shift+Q' };
    expect(matchTerminalAction(press({}), custom)).toBeNull();
    expect(matchTerminalAction(press({ key: 'Q', code: 'KeyQ' }), custom)).toBe('closePane');
    expect(matchTerminalAction(press({ key: 'W', code: 'KeyW' }), custom)).toBeNull();
  });
});

describe('defaultBindings', () => {
  it('leaves copy and paste to the Edit menu on macOS', () => {
    const mac = defaultBindings(true);
    expect(mac.copy).toBeNull();
    expect(mac.paste).toBeNull();
    expect(mac.closePane).toBe('Ctrl+Shift+W');
  });
});

describe('parseChord', () => {
  it('reads modifiers and the key token', () => {
    expect(parseChord('Ctrl+Shift+C')).toEqual({ ctrl: true, alt: false, shift: true, meta: false, key: 'C' });
  });

  it('rejects unknown modifiers and an empty key', () => {
    expect(parseChord('Hyper+C')).toBeNull();
    expect(parseChord('Ctrl+')).toBeNull();
    expect(parseChord('')).toBeNull();
  });
});

describe('formatChord', () => {
  it('shows keys as they are printed on the keyboard', () => {
    expect(formatChord('Alt+Shift+Equal', false)).toBe('Alt+Shift+=');
    expect(formatChord('Alt+Shift+Minus', false)).toBe('Alt+Shift+-');
    expect(formatChord('Ctrl+Digit1', false)).toBe('Ctrl+1');
    expect(formatChord('Ctrl+Shift+C', false)).toBe('Ctrl+Shift+C');
    expect(formatChord('Ctrl+F5', false)).toBe('Ctrl+F5');
  });

  it('uses the platform names for Alt and Meta', () => {
    expect(formatChord('Alt+Meta+K', true)).toBe('Option+Cmd+K');
    expect(formatChord('Alt+Meta+K', false)).toBe('Alt+Win+K');
  });
});

describe('validateChord', () => {
  it('accepts a free chord with a modifier', () => {
    expect(validateChord('closePane', 'Ctrl+Shift+Q', defaults)).toEqual({ ok: true, warning: null });
  });

  it('refuses a chord without Ctrl, Alt or Meta', () => {
    const result = validateChord('closePane', 'Shift+Q', defaults);
    expect(result.ok).toBe(false);
  });

  it('refuses a chord another action already uses', () => {
    expect(validateChord('closePane', 'Ctrl+Shift+C', defaults)).toEqual({
      ok: false,
      error: 'Already used by “Copy”'
    });
    // Its own current chord is not a conflict.
    expect(validateChord('copy', 'Ctrl+Shift+C', defaults).ok).toBe(true);
  });

  it('warns when a bare Ctrl+letter takes a control key from the shell', () => {
    const result = validateChord('closePane', 'Ctrl+W', defaults);
    expect(result.ok).toBe(true);
    expect(result.ok && result.warning).toContain('^W');
  });
});
