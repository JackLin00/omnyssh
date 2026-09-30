import { describe, expect, it } from 'vitest';
import { ByteHistory, HEX_ROW, SerialFormatter, mapEnter } from './serialFormat';

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
