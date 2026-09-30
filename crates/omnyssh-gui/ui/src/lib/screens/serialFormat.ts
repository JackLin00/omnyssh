// Turns received serial bytes into what a serial tab's xterm shows: decoded text, or
// a hex dump for devices whose output is binary. Stateful, because a chunk boundary
// can split a UTF-8 character or land mid-row of the dump.

export type SerialDisplay = 'text' | 'hex';
/** What the Enter key sends in a serial terminal. */
export type EnterKey = 'cr' | 'lf' | 'crlf';

/** Bytes per hex row, the 16 columns most serial tools print. */
export const HEX_ROW = 16;

const hex = (b: number): string => b.toString(16).toUpperCase().padStart(2, '0');

export class SerialFormatter {
  // Not fatal: bytes that are not UTF-8 show as U+FFFD instead of throwing.
  private decoder = new TextDecoder('utf-8');
  private column = 0;

  constructor(private display: SerialDisplay) {}

  /** Format one received chunk. Call in arrival order. */
  push(bytes: Uint8Array): string {
    if (this.display === 'text') return this.decoder.decode(bytes, { stream: true });
    let out = '';
    for (const b of bytes) {
      this.column = (this.column + 1) % HEX_ROW;
      out += hex(b) + (this.column === 0 ? '\r\n' : ' ');
    }
    return out;
  }

  /** Start over in `display`, before re-rendering the history. */
  reset(display: SerialDisplay): void {
    this.display = display;
    this.decoder = new TextDecoder('utf-8');
    this.column = 0;
  }
}

/** The received bytes kept for a re-render, at most `limit` of them. Whole chunks
 *  are dropped from the front, but the newest one is always kept. Eviction is amortized O(1). */
export class ByteHistory {
  private chunks: Uint8Array[] = [];
  private head = 0;
  private size = 0;

  constructor(private readonly limit: number) {}

  push(chunk: Uint8Array): void {
    this.chunks.push(chunk);
    this.size += chunk.length;
    while (this.size > this.limit && this.chunks.length - this.head > 1) {
      this.size -= this.chunks[this.head].length;
      this.head += 1;
    }
    if (this.head * 2 >= this.chunks.length) {
      this.chunks = this.chunks.slice(this.head);
      this.head = 0;
    }
  }

  all(): Uint8Array[] {
    return this.chunks.slice(this.head);
  }

  clear(): void {
    this.chunks = [];
    this.head = 0;
    this.size = 0;
  }
}

const ENTER: Record<EnterKey, string> = { cr: '\r', lf: '\n', crlf: '\r\n' };

/** xterm sends Enter as CR; rewrite it to what the device expects. */
export function mapEnter(data: string, enter: EnterKey): string {
  return data.replace(/\r/g, ENTER[enter]);
}
