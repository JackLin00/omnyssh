// The frontend half of session logs and exports (plan N): default file names, the
// log's header line, and the text of an xterm buffer for an export. Pure, so they are
// unit-tested without a terminal or a backend.

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

/** `YYYY-MM-DD` and `HH:MM:SS` in local time. */
function parts(at: Date): { date: string; time: string } {
  return {
    date: `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`,
    time: `${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())}`
  };
}

/** `name` with every character a file name cannot hold on Windows replaced by `_`,
 *  and without the trailing dots and spaces Windows drops. */
export function safeFileName(name: string): string {
  const cleaned = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').trim().replace(/[. ]+$/, '');
  return cleaned || 'session';
}

/** `web-1_2026-10-06_14-30-05.log` for a log, `…_export.txt` for an export. */
export function defaultFileName(name: string, at: Date, kind: 'log' | 'export'): string {
  const { date, time } = parts(at);
  const stem = `${safeFileName(name)}_${date}_${time.replace(/:/g, '-')}`;
  return kind === 'log' ? `${stem}.log` : `${stem}_export.txt`;
}

/** `# OmnySSH log: web-1 (deploy@10.0.0.5:22), started 2026-10-06 14:30:05`. */
export function logHeader(name: string, detail: string | null, at: Date): string {
  const { date, time } = parts(at);
  return `# OmnySSH log: ${name}${detail ? ` (${detail})` : ''}, started ${date} ${time}`;
}

/** What an SSH log's header says the session is connected to: `deploy@10.0.0.5:22`. */
export function sshLogDetail(
  host: { user: string; hostname: string; port: number } | undefined
): string | null {
  if (!host) return null;
  return `${host.user ? `${host.user}@` : ''}${host.hostname}:${host.port}`;
}

/** A serial tab's name for its files: the saved device's name, or the bare port for a
 *  tab opened from the Serial dialog (whose label is `COM3 · 115200`). */
export function serialLogName(tabLabel: string, port: string): string {
  return tabLabel.startsWith(`${port} · `) ? port : tabLabel;
}

/** `1.2 MB`, for the log button's tooltip. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** The part of an xterm buffer line `bufferText` reads. */
export interface BufferLineLike {
  readonly isWrapped: boolean;
  translateToString(trimRight?: boolean): string;
}

/** The part of an xterm buffer `bufferText` reads. */
export interface BufferLike {
  readonly length: number;
  getLine(y: number): BufferLineLike | undefined;
}

/** Everything in `buf`, scrollback and screen: one line per logical line (a row the
 *  terminal soft-wrapped is joined back to the one before it), trailing spaces and the
 *  empty rows below the last output dropped, each line ended by `\n`. */
export function bufferText(buf: BufferLike): string {
  const lines: string[] = [];
  for (let y = 0; y < buf.length; y++) {
    const line = buf.getLine(y);
    if (!line) continue;
    // A row that wraps on keeps its trailing blanks: they are part of the line.
    const text = line.translateToString(!buf.getLine(y + 1)?.isWrapped);
    if (line.isWrapped && lines.length > 0) lines[lines.length - 1] += text;
    else lines.push(text);
  }
  const trimmed = lines.map((l) => l.replace(/ +$/, ''));
  while (trimmed.length > 0 && trimmed[trimmed.length - 1] === '') trimmed.pop();
  return trimmed.map((l) => `${l}\n`).join('');
}

/** Whether the Start/Stop logging toggle should be disabled. A session that can no
 *  longer start a new recording (disconnected, port closed) can still be mid-way
 *  through stopping one it already started — e.g. right as the session ends, before
 *  its `log-stopped` lands — so Stop must stay reachable the whole time `logging` is
 *  true; only disabled while neither starting nor stopping is possible. */
export function logButtonDisabled(canStart: boolean, logging: boolean, picking: boolean): boolean {
  return (!canStart && !logging) || picking;
}
