import { describe, expect, it } from 'vitest';
import { ByteHistory, HEX_ROW, SerialFormatter, formatTimestamp, mapEnter } from './serialFormat';

const bytes = (...b: number[]): Uint8Array => Uint8Array.from(b);

describe('SerialFormatter', () => {
  it('decodes text, even when a UTF-8 character is split across chunks', () => {
    const f = new SerialFormatter('text');
    // 你 = E4 BD A0
    expect(f.push(bytes(0xe4, 0xbd)) + f.push(bytes(0xa0))).toBe('你');
  });

  it('prints hex as uppercase byte pairs separated by spaces', () => {
    const f = new SerialFormatter('hex');
    expect(f.push(bytes(0x48, 0x0a, 0xff))).toBe('48 0A FF ');
  });

  it('breaks a hex row every HEX_ROW bytes, across chunks', () => {
    const f = new SerialFormatter('hex');
    const out = f.push(new Uint8Array(10)) + f.push(new Uint8Array(10));
    const rows = out.split('\r\n');
    expect(rows[0]).toBe('00 '.repeat(HEX_ROW - 1) + '00');
    expect(rows[1]).toBe('00 '.repeat(20 - HEX_ROW));
  });

  it('reset switches the display and starts a fresh row', () => {
    const f = new SerialFormatter('hex');
    f.push(new Uint8Array(5));
    f.reset('hex');
    const out = f.push(new Uint8Array(HEX_ROW));
    expect(out.endsWith('00\r\n')).toBe(true);
    f.reset('text');
    expect(f.push(bytes(0x41))).toBe('A');
  });

  it('handles invalid UTF-8 gracefully with replacement character', () => {
    const f = new SerialFormatter('text');
    const result = f.push(bytes(0xff));
    expect(result).toContain('�');
    expect(result).not.toThrow;
  });
});

describe('ByteHistory', () => {
  it('drops the oldest chunks past the limit but always keeps the newest', () => {
    const h = new ByteHistory(4);
    h.push(bytes(1, 2));
    h.push(bytes(3, 4));
    h.push(bytes(5, 6, 7));
    expect(h.all()).toEqual([bytes(5, 6, 7)]);
    h.push(bytes(...new Array(10).fill(9)));
    expect(h.all()).toHaveLength(1);
  });

  it('clear empties it', () => {
    const h = new ByteHistory(100);
    h.push(bytes(1));
    h.clear();
    expect(h.all()).toEqual([]);
  });

  it('handles many small chunks efficiently and preserves last 100 bytes in order', () => {
    const h = new ByteHistory(100);
    // Push 10,000 one-byte chunks
    for (let i = 0; i < 10_000; i++) {
      h.push(bytes((i % 256)));
    }
    const all = h.all();
    const combined = new Uint8Array(all.reduce((sum, arr) => sum + arr.length, 0));
    let offset = 0;
    for (const chunk of all) {
      combined.set(chunk, offset);
      offset += chunk.length;
    }
    expect(combined.length).toBe(100);
    // Verify it contains the last 100 bytes in order (9900-9999 mod 256)
    for (let i = 0; i < 100; i++) {
      expect(combined[i]).toBe((9900 + i) % 256);
    }
  });
});

describe('mapEnter', () => {
  it('rewrites the CR xterm sends for Enter', () => {
    expect(mapEnter('AT\r', 'cr')).toBe('AT\r');
    expect(mapEnter('AT\r', 'lf')).toBe('AT\n');
    expect(mapEnter('AT\r', 'crlf')).toBe('AT\r\n');
  });

  it('leaves other characters alone', () => {
    expect(mapEnter('ls -l', 'crlf')).toBe('ls -l');
  });
});

// 2026-10-03 12:34:56.789 local time.
const AT = new Date(2026, 9, 3, 12, 34, 56, 789).getTime();
const LATER = AT + 1000; // 12:34:57.789
const DIM = '\x1b[90m';
const UNDIM = '\x1b[39m';
const ts = (s: string) => `${DIM}[${s}]${UNDIM} `;
const enc = (s: string) => new TextEncoder().encode(s);

describe('formatTimestamp', () => {
  it('reads as local [HH:MM:SS.mmm]', () => {
    expect(formatTimestamp(AT)).toBe('[12:34:56.789]');
    expect(formatTimestamp(new Date(2026, 0, 1, 1, 2, 3, 4).getTime())).toBe('[01:02:03.004]');
  });
});

describe('SerialFormatter with timestamps', () => {
  it('stamps each text line with when its first character arrived', () => {
    const f = new SerialFormatter('text', true);
    expect(f.push(enc('boot\r\nwifi '), AT)).toBe(`${ts('12:34:56.789')}boot\r\n${ts('12:34:56.789')}wifi `);
    // The line continues in the next chunk: no new stamp until after the newline.
    expect(f.push(enc('up\r\nok'), LATER)).toBe(`up\r\n${ts('12:34:57.789')}ok`);
  });

  it('leaves blank lines unstamped', () => {
    const f = new SerialFormatter('text', true);
    expect(f.push(enc('a\r\n\r\nb'), AT)).toBe(`${ts('12:34:56.789')}a\r\n\r\n${ts('12:34:56.789')}b`);
  });

  it('never stamps hex, which stays one continuous dump', () => {
    const f = new SerialFormatter('hex', true);
    expect(f.push(Uint8Array.from([0x55, 0xaa]), AT)).toBe('55 AA ');
    expect(f.push(Uint8Array.from([0x01]), LATER)).toBe('01 ');
  });

  it('switches timestamps on and off with reset, keeping the plain output unchanged', () => {
    const f = new SerialFormatter('text', true);
    f.reset('text', false);
    expect(f.push(enc('x\n'), AT)).toBe('x\n');
    f.reset('hex', false);
    expect(f.push(Uint8Array.from([1]), AT)).toBe('01 ');
  });
});

describe('ByteHistory arrival times', () => {
  it('keeps when each chunk arrived', () => {
    const h = new ByteHistory(100);
    h.push(Uint8Array.from([1]), AT);
    h.push(Uint8Array.from([2]), LATER);
    expect(h.entries()).toEqual([
      { bytes: Uint8Array.from([1]), at: AT },
      { bytes: Uint8Array.from([2]), at: LATER }
    ]);
  });

  it('drops times along with evicted chunks', () => {
    const h = new ByteHistory(2);
    h.push(Uint8Array.from([1, 2]), AT);
    h.push(Uint8Array.from([3, 4]), LATER);
    expect(h.entries()).toEqual([{ bytes: Uint8Array.from([3, 4]), at: LATER }]);
  });
});
