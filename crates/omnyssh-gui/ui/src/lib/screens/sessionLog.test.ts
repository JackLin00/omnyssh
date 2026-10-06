import { describe, expect, it } from 'vitest';
import {
  bufferText,
  defaultFileName,
  formatBytes,
  logButtonDisabled,
  logHeader,
  safeFileName,
  serialLogName,
  sshLogDetail,
  type BufferLike
} from './sessionLog';

// Built from local fields and read back the same way, so these pass in any time zone.
const AT = new Date(2026, 9, 6, 14, 3, 5);

describe('default file names', () => {
  it('stamp the name with the local date and time', () => {
    expect(defaultFileName('web-1', AT, 'log')).toBe('web-1_2026-10-06_14-03-05.log');
    expect(defaultFileName('web-1', AT, 'export')).toBe('web-1_2026-10-06_14-03-05_export.txt');
  });

  it('replace the characters Windows forbids in a file name', () => {
    expect(safeFileName('a<b>c:d"e/f\\g|h?i*j')).toBe('a_b_c_d_e_f_g_h_i_j');
    expect(safeFileName('tab\there')).toBe('tab_here');
    expect(defaultFileName('prod/db:5432', AT, 'log')).toBe('prod_db_5432_2026-10-06_14-03-05.log');
  });

  it('drop trailing dots and spaces, and never come out empty', () => {
    expect(safeFileName('  board v2. . ')).toBe('board v2');
    expect(safeFileName('...')).toBe('session');
    expect(safeFileName('')).toBe('session');
  });

  it('keep letters of any script', () => {
    expect(safeFileName('测试板 · COM3')).toBe('测试板 · COM3');
  });
});

describe('logHeader', () => {
  it('names the session, what it connects to, and when the log started', () => {
    expect(logHeader('web-1', 'deploy@10.0.0.5:22', AT)).toBe(
      '# OmnySSH log: web-1 (deploy@10.0.0.5:22), started 2026-10-06 14:03:05'
    );
    expect(logHeader('web-1', null, AT)).toBe('# OmnySSH log: web-1, started 2026-10-06 14:03:05');
  });
});

describe('sshLogDetail', () => {
  it('is user@host:port, without a user when there is none', () => {
    expect(sshLogDetail({ user: 'deploy', hostname: '10.0.0.5', port: 22 })).toBe('deploy@10.0.0.5:22');
    expect(sshLogDetail({ user: '', hostname: 'nas', port: 2222 })).toBe('nas:2222');
    expect(sshLogDetail(undefined)).toBeNull();
  });
});

describe('serialLogName', () => {
  it('is the bare port for a tab opened from the Serial dialog', () => {
    expect(serialLogName('COM3 · 115200', 'COM3')).toBe('COM3');
  });

  it('is the device name for a saved device', () => {
    expect(serialLogName('Bench PSU', 'COM7')).toBe('Bench PSU');
  });
});

describe('formatBytes', () => {
  it('scales to KB and MB', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(1536)).toBe('1.5 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
  });
});

/** A buffer of `cols`-wide rows, as xterm stores them: padded with spaces. */
function buffer(cols: number, rows: { text: string; wrapped?: boolean }[]): BufferLike {
  const lines = rows.map((r) => ({
    isWrapped: r.wrapped ?? false,
    translateToString: (trim?: boolean) => (trim ? r.text.replace(/ +$/, '') : r.text.padEnd(cols))
  }));
  return { length: lines.length, getLine: (y) => lines[y] };
}

describe('bufferText', () => {
  it('joins soft-wrapped rows back into one line, keeping the space at the wrap', () => {
    const buf = buffer(10, [
      { text: '$ echo one' },
      { text: 'a long lin' },
      { text: 'e of text ', wrapped: true },
      { text: 'here', wrapped: true },
      { text: '$' }
    ]);
    expect(bufferText(buf)).toBe('$ echo one\na long line of text here\n$\n');
  });

  it('drops trailing spaces and the empty rows below the output', () => {
    const buf = buffer(8, [{ text: 'top   ' }, { text: '' }, { text: 'end' }, { text: '' }, { text: '' }]);
    expect(bufferText(buf)).toBe('top\n\nend\n');
  });

  it('is empty for an empty terminal', () => {
    expect(bufferText(buffer(8, [{ text: '' }, { text: '' }]))).toBe('');
  });
});

describe('logButtonDisabled', () => {
  it('stays enabled to stop while recording, even once a new one could not start', () => {
    expect(logButtonDisabled(false, true, false)).toBe(false);
    expect(logButtonDisabled(true, true, false)).toBe(false);
  });

  it('is disabled when neither starting nor stopping is possible', () => {
    expect(logButtonDisabled(false, false, false)).toBe(true);
  });

  it('is enabled to start when nothing is being logged yet', () => {
    expect(logButtonDisabled(true, false, false)).toBe(false);
  });

  it('is disabled while the save dialog is open, logging or not', () => {
    expect(logButtonDisabled(true, false, true)).toBe(true);
    expect(logButtonDisabled(false, true, true)).toBe(true);
  });
});
