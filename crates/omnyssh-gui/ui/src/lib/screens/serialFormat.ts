// Turns received serial bytes into what a serial tab's xterm shows: decoded text, or
// a hex dump for devices whose output is binary. Stateful, because a chunk boundary
// can split a UTF-8 character or land mid-row of the dump.

export type SerialDisplay = 'text' | 'hex';
/** What the Enter key sends in a serial terminal. */
export type EnterKey = 'cr' | 'lf' | 'crlf';

/** Bytes per hex row, the 16 columns most serial tools print. */
export const HEX_ROW = 16;

// Precomputed so the hot path does no per-byte string formatting.
const HEX_BYTE = Array.from({ length: 256 }, (_, b) =>
  b.toString(16).toUpperCase().padStart(2, '0')
);

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

/** `[HH:MM:SS.mmm]` in local time, for a serial monitor line. */
export function formatTimestamp(at: number): string {
  const d = new Date(at);
  return `[${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}]`;
}

// Stamps are dim (bright black) and give the foreground back after, so a device's own
// colours on the rest of the line are untouched.
const stamp = (at: number) => `\x1b[90m${formatTimestamp(at)}\x1b[39m `;

export class SerialFormatter {
  // Not fatal: bytes that are not UTF-8 show as U+FFFD instead of throwing.
  private decoder = new TextDecoder('utf-8');
  private column = 0;
  /** Text mode: the next printable character starts a line and gets a stamp. */
  private lineStart = true;

  constructor(
    private display: SerialDisplay,
    private timestamps = false
  ) {}

  /** Format one received chunk, which arrived at `at` (ms since the epoch). Call in arrival order. */
  push(bytes: Uint8Array, at: number = Date.now()): string {
    if (this.display === 'text') {
      const text = this.decoder.decode(bytes, { stream: true });
      return this.timestamps ? this.stampLines(text, at) : text;
    }
    // Hex stays one continuous dump: stamps are a text-display feature.
    return this.hexRun(bytes);
  }

  /** Start over in `display`, with or without timestamps (which only text shows), before
   *  re-rendering the history. */
  reset(display: SerialDisplay, timestamps: boolean = this.timestamps): void {
    this.display = display;
    this.timestamps = timestamps;
    this.decoder = new TextDecoder('utf-8');
    this.column = 0;
    this.lineStart = true;
  }

  private hexRun(bytes: Uint8Array): string {
    let out = '';
    for (const b of bytes) {
      this.column = (this.column + 1) % HEX_ROW;
      out += HEX_BYTE[b] + (this.column === 0 ? '\r\n' : ' ');
    }
    return out;
  }

  private stampLines(text: string, at: number): string {
    let out = '';
    for (const ch of text) {
      if (this.lineStart && ch !== '\r' && ch !== '\n') {
        out += stamp(at);
        this.lineStart = false;
      }
      out += ch;
      if (ch === '\n') this.lineStart = true;
    }
    return out;
  }
}

/** The received bytes kept for a re-render, at most `limit` of them. Whole chunks
 *  are dropped from the front, but the newest one is always kept. Eviction is amortized O(1). */
export class ByteHistory {
  private chunks: Uint8Array[] = [];
  private times: number[] = [];
  private head = 0;
  private size = 0;

  constructor(private readonly limit: number) {}

  push(chunk: Uint8Array, at: number = Date.now()): void {
    this.chunks.push(chunk);
    this.times.push(at);
    this.size += chunk.length;
    while (this.size > this.limit && this.chunks.length - this.head > 1) {
      this.size -= this.chunks[this.head].length;
      this.head += 1;
    }
    if (this.head * 2 >= this.chunks.length) {
      this.chunks = this.chunks.slice(this.head);
      this.times = this.times.slice(this.head);
      this.head = 0;
    }
  }

  all(): Uint8Array[] {
    return this.chunks.slice(this.head);
  }

  /** Every kept chunk with when it arrived, oldest first. */
  entries(): { bytes: Uint8Array; at: number }[] {
    return this.chunks.slice(this.head).map((bytes, i) => ({ bytes, at: this.times[this.head + i] }));
  }

  clear(): void {
    this.chunks = [];
    this.times = [];
    this.head = 0;
    this.size = 0;
  }
}

const ENTER: Record<EnterKey, string> = { cr: '\r', lf: '\n', crlf: '\r\n' };

/** xterm sends Enter as CR; rewrite it to what the device expects. */
export function mapEnter(data: string, enter: EnterKey): string {
  return data.replace(/\r/g, ENTER[enter]);
}
