import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

// Hoisted with the mock below, which runs before the imports.
const { pickSavePath, logStart, saveTextFile } = vi.hoisted(() => ({
  pickSavePath: vi.fn(),
  logStart: vi.fn(),
  saveTextFile: vi.fn()
}));
vi.mock('$lib/ipc/commands', () => ({ pickSavePath, logStart, saveTextFile }));

import {
  applyLogStopped,
  exportText,
  fileName,
  loggingLabel,
  logStarted,
  sessionLogs,
  startSessionLog
} from './sessionLogs';
import { lastError, statusNotice } from './notifications';

const target = {
  sessionId: 7,
  name: 'web-1',
  detail: 'deploy@10.0.0.5:22',
  mode: 'text' as const,
  timestamps: true
};

describe('sessionLogs', () => {
  beforeEach(() => {
    sessionLogs.set(new Map());
    lastError.set(null);
    statusNotice.set(null);
    pickSavePath.mockReset();
    logStart.mockReset().mockResolvedValue(undefined);
    saveTextFile.mockReset().mockResolvedValue(undefined);
  });

  it('takes the file name from either kind of path', () => {
    expect(fileName('C:\\Users\\me\\Documents\\web-1.log')).toBe('web-1.log');
    expect(fileName('/home/me/web-1.log')).toBe('web-1.log');
  });

  it('starts a log where the dialog says, with a header and the timestamps choice', async () => {
    pickSavePath.mockResolvedValue('/logs/web-1.log');
    expect(await startSessionLog(target)).toBe(true);
    expect(pickSavePath).toHaveBeenCalledWith(expect.stringMatching(/^web-1_\d{4}-\d\d-\d\d_\d\d-\d\d-\d\d\.log$/), 'log');
    expect(logStart).toHaveBeenCalledWith(
      7,
      '/logs/web-1.log',
      true,
      'text',
      expect.stringMatching(/^# OmnySSH log: web-1 \(deploy@10\.0\.0\.5:22\), started /)
    );
    expect(get(sessionLogs).get(7)?.path).toBe('/logs/web-1.log');
  });

  it('does nothing when the dialog is cancelled', async () => {
    pickSavePath.mockResolvedValue(null);
    expect(await startSessionLog(target)).toBe(false);
    expect(logStart).not.toHaveBeenCalled();
    expect(get(sessionLogs).size).toBe(0);
  });

  it('a failed start is not recorded as logging', async () => {
    pickSavePath.mockResolvedValue('/logs/web-1.log');
    logStart.mockRejectedValue(new Error('That session has ended'));
    await expect(startSessionLog(target)).rejects.toThrow('That session has ended');
    expect(get(sessionLogs).size).toBe(0);
  });

  it('a log-stopped landing before logStart resolves is not overwritten by it', async () => {
    pickSavePath.mockResolvedValue('/logs/web-1.log');
    // The backend's own race guard (plan N review item 1) — or simply the session
    // ending — can report log-stopped before the logStart command that opened the
    // log even resolves: simulated here by firing it from inside the mocked
    // command, synchronously before that command's own promise settles.
    logStart.mockImplementation(async () => {
      applyLogStopped({ sessionId: 7, path: '/logs/web-1.log', bytes: 3, error: null });
    });
    expect(await startSessionLog(target)).toBe(true);
    // Not logging: the in-flight start must not resurrect a log already closed.
    expect(get(sessionLogs).size).toBe(0);
    expect(get(statusNotice)?.message).toBe('Saved log to /logs/web-1.log');
  });

  it('a stop drops the log and says where it was saved', () => {
    logStarted(7, '/logs/web-1.log');
    applyLogStopped({ sessionId: 7, path: '/logs/web-1.log', bytes: 10, error: null });
    expect(get(sessionLogs).size).toBe(0);
    expect(get(statusNotice)).toEqual({ message: 'Saved log to /logs/web-1.log', path: '/logs/web-1.log' });
    expect(get(lastError)).toBeNull();
  });

  it('a failed write reports why, as an error', () => {
    logStarted(7, '/logs/web-1.log');
    applyLogStopped({ sessionId: 7, path: '/logs/web-1.log', bytes: 10, error: 'No space left on device' });
    expect(get(sessionLogs).size).toBe(0);
    expect(get(lastError)).toBe('Logging to web-1.log stopped: No space left on device');
    expect(get(statusNotice)).toBeNull();
  });

  it('the status bar names the newest log and counts the rest', () => {
    expect(get(loggingLabel)).toBeNull();
    logStarted(1, '/logs/a.log', 1000);
    expect(get(loggingLabel)).toBe('Logging to a.log');
    logStarted(2, 'C:\\logs\\b.log', 2000);
    expect(get(loggingLabel)).toBe('Logging to b.log (+1)');
  });

  it('exports to the picked file and offers to show it', async () => {
    pickSavePath.mockResolvedValue('/logs/web-1_export.txt');
    expect(await exportText('web-1', 'line\n')).toBe(true);
    expect(pickSavePath).toHaveBeenCalledWith(expect.stringMatching(/_export\.txt$/), 'export');
    expect(saveTextFile).toHaveBeenCalledWith('/logs/web-1_export.txt', 'line\n');
    expect(get(statusNotice)?.path).toBe('/logs/web-1_export.txt');
  });
});
