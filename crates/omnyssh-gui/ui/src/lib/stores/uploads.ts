// Drag-and-drop uploads in terminal and serial tabs (plan Q), keyed by the backend
// session id. A tab starts one with `startUpload`; the backend's `upload-progress`
// and `upload-finished` move it along; the tab's end (`terminal-exited`, a serial
// exit, closing it) drops it with `endUpload`, because an upload whose session ends
// first gets no `upload-finished`. The terminal shows the result line itself (the
// backend prints it), so nothing here reports success.
import { writable, get } from 'svelte/store';
import type { UploadFinished, UploadProgress } from '$lib/bindings';
import { lastError } from './notifications';

export type UploadProtocol = 'zmodem' | 'ymodem' | 'xmodem' | 'sftp';

export interface UploadView {
  protocol: UploadProtocol;
  /** `starting` until the backend reports: the command is running (an SFTP
   *  connection may be logging in); `waiting` for the receiver (`rz`, `loady`). */
  phase: 'starting' | 'waiting' | 'sending';
  fileIndex: number;
  fileCount: number;
  fileName: string;
  sent: number;
  total: number;
  /** Bytes per second, smoothed; 0 until there are two samples of one file. */
  rate: number;
  /** The last sample the rate was taken from (ms, `sent`). */
  sampleAt: number;
  sampleSent: number;
}

export const uploads = writable<Map<number, UploadView>>(new Map());

const NAMES: Record<UploadProtocol, string> = {
  zmodem: 'ZMODEM',
  ymodem: 'YMODEM',
  xmodem: 'XMODEM',
  sftp: 'SFTP'
};

export function protocolName(p: UploadProtocol): string {
  return NAMES[p];
}

/** ZMODEM, YMODEM and XMODEM own the byte stream while they run: keystrokes would
 *  land in the middle of it. SFTP runs beside the shell. */
export function blocksInput(view: UploadView | undefined): boolean {
  return view != null && view.protocol !== 'sftp';
}

/** Whether a drop of `count` files can go by `protocol`, checked before anything is
 *  sent. The backend checks the rest (folders, sizes, names). */
export function dropError(protocol: UploadProtocol, count: number): string | null {
  return protocol === 'xmodem' && count > 1 ? 'XMODEM sends one file at a time' : null;
}

/** A note for the drop overlay: U-Boot's `loady` takes only the first file of a batch. */
export function dropNote(protocol: UploadProtocol, count: number): string | null {
  return protocol === 'ymodem' && count > 1
    ? 'A bootloader such as U-Boot takes only the first file'
    : null;
}

function set(id: number, view: UploadView | null): void {
  uploads.update((m) => {
    const next = new Map(m);
    if (view) next.set(id, view);
    else next.delete(id);
    return next;
  });
}

/** Starts an upload on backend session `id` through `send` (`terminalUpload` or
 *  `serialUpload`). Refusals go to the status bar; a cancel during an SFTP login
 *  comes back as the error `cancelled` and is not one. */
export async function startUpload(
  id: number,
  protocol: UploadProtocol,
  paths: string[],
  send: (id: number, protocol: UploadProtocol, paths: string[]) => Promise<void>
): Promise<void> {
  const refused = dropError(protocol, paths.length);
  if (refused) {
    lastError.set(refused);
    return;
  }
  if (get(uploads).has(id)) {
    lastError.set('An upload is already running here');
    return;
  }
  set(id, {
    protocol,
    phase: 'starting',
    fileIndex: 0,
    fileCount: paths.length,
    fileName: '',
    sent: 0,
    total: 0,
    rate: 0,
    sampleAt: 0,
    sampleSent: 0
  });
  try {
    await send(id, protocol, paths);
  } catch (e) {
    endUpload(id);
    const message = e instanceof Error ? e.message : String(e);
    if (message !== 'cancelled') lastError.set(`Upload: ${message}`);
  }
}

/** How much a new rate sample counts against the running rate. */
const RATE_WEIGHT = 0.3;

export function applyUploadProgress(p: UploadProgress, now: number = Date.now()): void {
  const view = get(uploads).get(p.sessionId);
  // Unknown: the tab has gone, or the upload ended already.
  if (!view) return;
  if (p.waiting) {
    set(p.sessionId, { ...view, phase: 'waiting' });
    return;
  }
  const sameFile = view.phase === 'sending' && view.fileIndex === p.fileIndex;
  let rate = sameFile ? view.rate : 0;
  if (sameFile && now > view.sampleAt && p.sent > view.sampleSent) {
    const sample = ((p.sent - view.sampleSent) * 1000) / (now - view.sampleAt);
    rate = rate === 0 ? sample : rate + RATE_WEIGHT * (sample - rate);
  }
  set(p.sessionId, {
    ...view,
    phase: 'sending',
    fileIndex: p.fileIndex,
    fileCount: p.fileCount,
    fileName: p.fileName,
    sent: p.sent,
    total: p.total,
    rate,
    sampleAt: now,
    sampleSent: p.sent
  });
}

/** The upload ended: the terminal already shows its result line. */
export function applyUploadFinished(f: UploadFinished): void {
  endUpload(f.sessionId);
}

/** Forget session `id`'s upload: it finished, or its session ended. */
export function endUpload(id: number | undefined): void {
  if (id != null && get(uploads).has(id)) set(id, null);
}

/** `512 B`, `1.2 KB`, `3.4 MB` (powers of 1024, like the backend's result line). */
export function formatSize(bytes: number): string {
  const units = ['KB', 'MB', 'GB', 'TB'];
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1023.95 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(1)} ${units[unit]}`;
}

/** What the progress bar shows: its text, and how full it is (0..1, or null while
 *  there is nothing to measure). */
export function progressLine(view: UploadView): { text: string; fraction: number | null } {
  const name = protocolName(view.protocol);
  if (view.phase === 'starting') {
    return { text: view.protocol === 'sftp' ? 'Connecting for SFTP…' : `Starting ${name}…`, fraction: null };
  }
  if (view.phase === 'waiting') {
    return {
      text: view.protocol === 'zmodem' ? 'Waiting for rz…' : 'Waiting for the receiver…',
      fraction: null
    };
  }
  const fraction = view.total > 0 ? Math.min(1, view.sent / view.total) : 1;
  const parts = [view.fileName];
  if (view.fileCount > 1) parts.push(`${view.fileIndex + 1}/${view.fileCount}`);
  parts.push(`${Math.floor(fraction * 100)}%`);
  if (view.rate > 0) parts.push(`${formatSize(view.rate)}/s`);
  return { text: parts.join(' · '), fraction };
}
