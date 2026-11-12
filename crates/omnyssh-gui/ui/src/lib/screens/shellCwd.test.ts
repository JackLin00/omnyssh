import { describe, expect, it } from 'vitest';
import { createCwdTracker, CWD_SETUP, parseOsc7, sftpFolder } from './shellCwd';

describe('parseOsc7', () => {
  it('reads the host and the folder', () => {
    expect(parseOsc7('file://web-1/var/log')).toEqual({ host: 'web-1', path: '/var/log' });
    expect(parseOsc7('file:///srv')).toEqual({ host: '', path: '/srv' });
  });

  it('decodes percent escapes, and takes a path that is not encoded as it is', () => {
    expect(parseOsc7('file://h/home/me/My%20Files')?.path).toBe('/home/me/My Files');
    expect(parseOsc7('file://h/home/me/My Files')?.path).toBe('/home/me/My Files');
    expect(parseOsc7('file://h/tmp/100%')?.path).toBe('/tmp/100%');
  });

  it('ignores anything else', () => {
    expect(parseOsc7('')).toBeNull();
    expect(parseOsc7('http://h/x')).toBeNull();
    expect(parseOsc7('file://h')).toBeNull();
  });
});

describe('createCwdTracker', () => {
  it('follows the folder the shell reports', () => {
    const t = createCwdTracker();
    expect(t.cwd()).toBeNull();
    t.report('file://web-1/home/me');
    t.report('file://web-1/var/log');
    expect(t.cwd()).toBe('/var/log');
  });

  it('forgets the folder while a shell on another machine reports, and picks it up again', () => {
    const t = createCwdTracker();
    t.report('file://web-1/srv');
    // `ssh db-1` from this shell: that folder is on db-1, not where SFTP connects.
    t.report('file://db-1/tmp');
    expect(t.cwd()).toBeNull();
    t.report('file://web-1/srv/app');
    expect(t.cwd()).toBe('/srv/app');
  });

  it('takes a report without a host as this machine', () => {
    const t = createCwdTracker();
    t.report('file://web-1/srv');
    t.report('file:///opt');
    expect(t.cwd()).toBe('/opt');
  });

  it('keeps the folder through a malformed report', () => {
    const t = createCwdTracker();
    t.report('file://web-1/srv');
    t.report('garbage');
    expect(t.cwd()).toBe('/srv');
  });
});

describe('sftpFolder', () => {
  it('prefers the shell folder, then the host setting, then home', () => {
    expect(sftpFolder('/var/log', '/srv/in')).toBe('/var/log');
    expect(sftpFolder(null, '/srv/in')).toBe('/srv/in');
    expect(sftpFolder(null, '  ')).toBe('~');
    expect(sftpFolder(null, null)).toBe('~');
  });
});

describe('CWD_SETUP', () => {
  it('is one line, kept out of the history, that ends with Enter', () => {
    expect(CWD_SETUP.startsWith(' ')).toBe(true);
    expect(CWD_SETUP.endsWith('\r')).toBe(true);
    expect(CWD_SETUP.slice(0, -1)).not.toMatch(/[\r\n]/);
    // bash and zsh both, set up only once per shell.
    expect(CWD_SETUP).toContain('PROMPT_COMMAND');
    expect(CWD_SETUP).toContain('precmd_functions');
    expect(CWD_SETUP).toContain('__OMNY_CWD');
  });
});
