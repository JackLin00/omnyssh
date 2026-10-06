import { derived, writable } from 'svelte/store';
import type { LogModeDto, LogStopped } from '$lib/bindings';
import { logStart, pickSavePath, saveTextFile } from '$lib/ipc/commands';
import { defaultFileName, logHeader } from '$lib/screens/sessionLog';
import { lastError, statusNotice } from './notifications';

// The session logs being written (plan N), by backend session id. The backend writes
// them; this store only knows which sessions record into which file, for the buttons
// and the status bar. Every stop — asked for, the session ending, a failed write —
// arrives as `log-stopped`.

export interface ActiveLog {
  path: string;
  /** When the log started (ms since the epoch); the newest one names the status bar. */
  startedAt: number;
}

export const sessionLogs = writable<Map<number, ActiveLog>>(new Map());

/** The file name at the end of `path`, either separator. */
export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

export function logStarted(sessionId: number, path: string, at: number = Date.now()): void {
  sessionLogs.update((m) => new Map(m).set(sessionId, { path, startedAt: at }));
}

/** Ids with a `logStart` command in flight: `log-stopped` for one of these can land
 *  before the command resolves — the session can end right after the log opens
 *  (backend's own race guard), racing the awaited `logStart` here. */
const starting = new Set<number>();
/** Ids from `starting` a `log-stopped` landed on while still in flight, so the
 *  resolved `logStart` below knows not to add a now-already-stopped session back
 *  into `sessionLogs`. */
const stoppedWhileStarting = new Set<number>();

export function applyLogStopped(p: LogStopped): void {
  if (starting.has(p.sessionId)) stoppedWhileStarting.add(p.sessionId);
  sessionLogs.update((m) => {
    if (!m.has(p.sessionId)) return m;
    const next = new Map(m);
    next.delete(p.sessionId);
    return next;
  });
  if (p.error) lastError.set(`Logging to ${fileName(p.path)} stopped: ${p.error}`);
  else statusNotice.set({ message: `Saved log to ${p.path}`, path: p.path });
}

/** `Logging to web-1_….log`, plus how many more are being written; `null` when none. */
export const loggingLabel = derived(sessionLogs, (m): string | null => {
  if (m.size === 0) return null;
  const newest = [...m.values()].reduce((a, b) => (b.startedAt > a.startedAt ? b : a));
  return `Logging to ${fileName(newest.path)}${m.size > 1 ? ` (+${m.size - 1})` : ''}`;
});

export interface LogTarget {
  sessionId: number;
  /** The tab's name, for the file name and the header. */
  name: string;
  /** What it is connected to (`deploy@10.0.0.5:22`, `COM3 · 115200 8N1`), for the header. */
  detail: string | null;
  mode: LogModeDto;
  timestamps: boolean;
}

/** Ask where to save, then start logging. Resolves `false` when the user cancelled the
 *  dialog; a failure rejects. */
export async function startSessionLog(t: LogTarget): Promise<boolean> {
  const path = await pickSavePath(defaultFileName(t.name, new Date(), 'log'), 'log');
  if (!path) return false;
  starting.add(t.sessionId);
  let stoppedMidFlight = false;
  try {
    await logStart(t.sessionId, path, t.timestamps, t.mode, logHeader(t.name, t.detail, new Date()));
  } finally {
    starting.delete(t.sessionId);
    stoppedMidFlight = stoppedWhileStarting.delete(t.sessionId);
  }
  // A log-stopped for this id already arrived (and already updated the status bar)
  // while the command was in flight — do not resurrect it as still logging.
  if (!stoppedMidFlight) logStarted(t.sessionId, path);
  return true;
}

/** Ask where to save, then write `text` there. Resolves `false` when cancelled. */
export async function exportText(name: string, text: string): Promise<boolean> {
  const path = await pickSavePath(defaultFileName(name, new Date(), 'export'), 'export');
  if (!path) return false;
  await saveTextFile(path, text);
  statusNotice.set({ message: `Exported to ${path}`, path });
  return true;
}
