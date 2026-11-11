import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import {
  applyUploadFinished,
  applyUploadProgress,
  blocksInput,
  dropError,
  dropNote,
  endUpload,
  formatSize,
  progressLine,
  startUpload,
  uploads,
  type UploadProtocol
} from './uploads';
import { lastError } from './notifications';

const progress = (over: Partial<Parameters<typeof applyUploadProgress>[0]> = {}) => ({
  sessionId: 7,
  waiting: false,
  fileIndex: 0,
  fileCount: 1,
  fileName: 'a.bin',
  sent: 0,
  total: 1000,
  ...over
});

/** A send that records what it was asked and resolves. */
function recorder() {
  const calls: [number, UploadProtocol, string[]][] = [];
  const send = (id: number, p: UploadProtocol, paths: string[]) => {
    calls.push([id, p, paths]);
    return Promise.resolve();
  };
  return { calls, send };
}

beforeEach(() => {
  uploads.set(new Map());
  lastError.set(null);
});

describe('startUpload', () => {
  it('starts in the starting phase and sends the drop', async () => {
    const { calls, send } = recorder();
    await startUpload(7, 'zmodem', ['/a.bin', '/b.bin'], send);
    expect(calls).toEqual([[7, 'zmodem', ['/a.bin', '/b.bin']]]);
    const view = get(uploads).get(7)!;
    expect(view.phase).toBe('starting');
    expect(view.fileCount).toBe(2);
  });

  it('refuses several files for XMODEM before sending anything', async () => {
    const { calls, send } = recorder();
    await startUpload(7, 'xmodem', ['/a', '/b'], send);
    expect(calls).toEqual([]);
    expect(get(lastError)).toBe('XMODEM sends one file at a time');
    expect(get(uploads).has(7)).toBe(false);
  });

  it('refuses a second upload in the same session', async () => {
    const { calls, send } = recorder();
    await startUpload(7, 'zmodem', ['/a'], send);
    await startUpload(7, 'zmodem', ['/b'], send);
    expect(calls).toHaveLength(1);
    expect(get(lastError)).toBe('An upload is already running here');
  });

  it('drops the upload and reports a refusal from the backend', async () => {
    await startUpload(7, 'zmodem', ['/dir'], () =>
      Promise.reject(new Error('folders cannot be uploaded: dir'))
    );
    expect(get(uploads).has(7)).toBe(false);
    expect(get(lastError)).toBe('Upload: folders cannot be uploaded: dir');
  });

  it('treats a cancel during the SFTP login as a cancel, not an error', async () => {
    await startUpload(7, 'sftp', ['/a'], () => Promise.reject(new Error('cancelled')));
    expect(get(uploads).has(7)).toBe(false);
    expect(get(lastError)).toBeNull();
  });
});

describe('progress', () => {
  it('moves from waiting to sending and computes a smoothed rate', async () => {
    await startUpload(7, 'zmodem', ['/a.bin'], recorder().send);
    applyUploadProgress(progress({ waiting: true, fileName: '', total: 0 }), 0);
    expect(get(uploads).get(7)!.phase).toBe('waiting');
    applyUploadProgress(progress({ sent: 0 }), 1000);
    expect(get(uploads).get(7)!.rate).toBe(0);
    applyUploadProgress(progress({ sent: 100 }), 2000);
    expect(get(uploads).get(7)!.rate).toBe(100);
    applyUploadProgress(progress({ sent: 400 }), 3000);
    // 100 + 0.3 * (300 - 100)
    expect(get(uploads).get(7)!.rate).toBeCloseTo(160);
  });

  it('starts the rate over on the next file, and ignores a resend going back', async () => {
    await startUpload(7, 'zmodem', ['/a', '/b'], recorder().send);
    applyUploadProgress(progress({ sent: 0, fileCount: 2 }), 0);
    applyUploadProgress(progress({ sent: 500, fileCount: 2 }), 1000);
    applyUploadProgress(progress({ sent: 300, fileCount: 2 }), 1500);
    expect(get(uploads).get(7)!.rate).toBe(500);
    applyUploadProgress(progress({ fileIndex: 1, fileName: 'b', sent: 10, fileCount: 2 }), 2000);
    expect(get(uploads).get(7)!.rate).toBe(0);
  });

  it('ignores progress for a session it does not know', () => {
    applyUploadProgress(progress({ sessionId: 99 }));
    expect(get(uploads).size).toBe(0);
  });

  it('ends on upload-finished and on endUpload', async () => {
    await startUpload(7, 'zmodem', ['/a'], recorder().send);
    applyUploadFinished({ sessionId: 7, ok: true, message: 'uploaded a (1 B)' });
    expect(get(uploads).has(7)).toBe(false);
    await startUpload(8, 'sftp', ['/a'], recorder().send);
    endUpload(8);
    endUpload(undefined);
    expect(get(uploads).size).toBe(0);
  });
});

describe('display', () => {
  it('blocks input for the protocols that own the stream', () => {
    const base = { phase: 'sending' as const, fileIndex: 0, fileCount: 1, fileName: '', sent: 0, total: 0, rate: 0, sampleAt: 0, sampleSent: 0 };
    expect(blocksInput(undefined)).toBe(false);
    expect(blocksInput({ ...base, protocol: 'ymodem' })).toBe(true);
    expect(blocksInput({ ...base, protocol: 'sftp' })).toBe(false);
  });

  it('writes the progress line', () => {
    const base = { protocol: 'zmodem' as const, fileIndex: 1, fileCount: 3, fileName: 'b.bin', sent: 512, total: 1024, rate: 1536, sampleAt: 0, sampleSent: 0 };
    expect(progressLine({ ...base, phase: 'sending' })).toEqual({
      text: 'b.bin · 2/3 · 50% · 1.5 KB/s',
      fraction: 0.5
    });
    expect(progressLine({ ...base, phase: 'waiting' }).text).toBe('Waiting for rz…');
    expect(progressLine({ ...base, protocol: 'ymodem', phase: 'waiting' }).text).toBe('Waiting for the receiver…');
    expect(progressLine({ ...base, protocol: 'sftp', phase: 'starting' })).toEqual({
      text: 'Connecting for SFTP…',
      fraction: null
    });
    expect(progressLine({ ...base, phase: 'sending', fileCount: 1, total: 0, rate: 0 }).text).toBe('b.bin · 100%');
  });

  it('notes and refusals for the drop overlay', () => {
    expect(dropError('xmodem', 1)).toBeNull();
    expect(dropError('xmodem', 2)).not.toBeNull();
    expect(dropNote('ymodem', 2)).toContain('U-Boot');
    expect(dropNote('zmodem', 2)).toBeNull();
  });

  it('formats sizes like the backend', () => {
    expect(formatSize(1023)).toBe('1023 B');
    expect(formatSize(1024)).toBe('1.0 KB');
    expect(formatSize(1_048_575)).toBe('1.0 MB');
  });
});
