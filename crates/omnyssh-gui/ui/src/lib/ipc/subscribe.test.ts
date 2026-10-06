import { beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

type Listener = (e: { payload: unknown }) => void;

// Capture the callback each event is wired to, replacing the Tauri-backed
// bindings so the event-name -> handler mapping can be exercised off-runtime.
const { listeners } = vi.hoisted(() => ({ listeners: {} as Record<string, Listener> }));

vi.mock('$lib/bindings', () => {
  const channel = (name: string) => ({
    listen: (cb: Listener) => {
      listeners[name] = cb;
      return Promise.resolve(() => delete listeners[name]);
    }
  });
  return {
    events: {
      hostsLoaded: channel('hostsLoaded'),
      hostStatusChanged: channel('hostStatusChanged'),
      metricsUpdated: channel('metricsUpdated'),
      servicesDetected: channel('servicesDetected'),
      servicesFailed: channel('servicesFailed'),
      tunnelStatusChanged: channel('tunnelStatusChanged'),
      snippetResult: channel('snippetResult'),
      terminalExited: channel('terminalExited'),
      sftpConnected: channel('sftpConnected'),
      sftpDirListed: channel('sftpDirListed'),
      sftpOpDone: channel('sftpOpDone'),
      sftpDisconnected: channel('sftpDisconnected'),
      filePreview: channel('filePreview'),
      transferProgress: channel('transferProgress'),
      keySetupProgress: channel('keySetupProgress'),
      keySetupComplete: channel('keySetupComplete'),
      keySetupFailed: channel('keySetupFailed'),
      keySetupRollback: channel('keySetupRollback'),
      updateAvailable: channel('updateAvailable'),
      keyPassphraseRequired: channel('keyPassphraseRequired'),
      passwordRequired: channel('passwordRequired'),
      hostKeyChanged: channel('hostKeyChanged'),
      logStopped: channel('logStopped'),
      error: channel('error')
    }
  };
});

import { hosts } from '$lib/stores/hosts';
import { statuses } from '$lib/stores/statuses';
import { metrics } from '$lib/stores/metrics';
import { services } from '$lib/stores/services';
import { snippetRun, beginRun, clearRun } from '$lib/stores/snippets';
import { sessions } from '$lib/stores/sessions';
import { sftp } from '$lib/stores/sftp';
import { lastError } from '$lib/stores/notifications';
import { passphrasePrompt, passphraseQueue } from '$lib/stores/passphrase';
import { passwordPrompt, passwordQueue } from '$lib/stores/password';
import { hostKeyPrompt, hostKeyQueue } from '$lib/stores/hostKey';
import { sessionLogs, logStarted } from '$lib/stores/sessionLogs';
import { statusNotice } from '$lib/stores/notifications';
import { startEventBridge } from './subscribe';

describe('startEventBridge', () => {
  beforeEach(() => {
    hosts.set([]);
    statuses.set(new Map());
    metrics.set(new Map());
    services.set(new Map());
    lastError.set(null);
    passphraseQueue.set([]);
    passwordQueue.set([]);
    hostKeyQueue.set([]);
    clearRun();
  });

  it('routes each event to its matching store', async () => {
    await startEventBridge();

    listeners.hostsLoaded({
      payload: [
        { name: 'web-1', hostname: '10.0.0.1', user: 'root', port: 22, tags: [], source: 'manual', hasKey: false }
      ]
    });
    listeners.hostStatusChanged({ payload: { hostName: 'web-1', status: { kind: 'connected' } } });
    listeners.metricsUpdated({
      payload: { hostName: 'web-1', metrics: { cpuPercent: 5, topProcesses: [], ageSeconds: 0 } }
    });
    listeners.servicesDetected({
      payload: { hostName: 'web-1', services: [{ kind: 'redis', metrics: [] }] }
    });
    beginRun('deploy', ['web-1']);
    listeners.snippetResult({
      payload: { hostName: 'web-1', snippetName: 'deploy', ok: true, output: 'done' }
    });
    listeners.error({ payload: { message: 'nope' } });
    listeners.keyPassphraseRequired({ payload: { hostName: 'web-1', keyPath: '/k/id_ed25519' } });

    expect(get(hosts)).toHaveLength(1);
    expect(get(statuses).get('web-1')).toEqual({ kind: 'connected' });
    expect(get(metrics).get('web-1')?.cpuPercent).toBe(5);
    expect(get(services).get('web-1')).toEqual({ kind: 'detected', services: [{ kind: 'redis', metrics: [] }] });
    expect(get(snippetRun)?.entries[0]).toEqual({ hostName: 'web-1', pending: false, ok: true, output: 'done' });
    expect(get(lastError)).toBe('nope');
    expect(get(passphrasePrompt)).toEqual({ hostName: 'web-1', keyPath: '/k/id_ed25519' });
  });

  it('queues password prompts in order', async () => {
    await startEventBridge();
    const first = { requestId: 7, hostName: 'nas', login: 'admin@10.0.0.5', retry: false, newHostKey: null };
    const second = { ...first, requestId: 8, hostName: 'db' };
    listeners.passwordRequired({ payload: first });
    listeners.passwordRequired({ payload: second });
    expect(get(passwordPrompt)).toEqual(first);
    expect(get(passwordQueue)).toEqual([first, second]);
  });

  it('queues host-key questions in order', async () => {
    await startEventBridge();
    const key = { keyType: 'ssh-ed25519', fingerprint: 'SHA256:x' };
    const first = { requestId: 5, hostName: 'nas', host: 'nas.lan', port: 22, jumpFor: null, file: '/k', saved: [key], offered: key };
    const second = { ...first, requestId: 6, hostName: 'db' };
    listeners.hostKeyChanged({ payload: first });
    listeners.hostKeyChanged({ payload: second });
    expect(get(hostKeyPrompt)).toEqual(first);
    expect(get(hostKeyQueue)).toEqual([first, second]);
  });

  it('log-stopped drops the log and says where it was saved', async () => {
    await startEventBridge();
    logStarted(42, '/logs/web-1.log');
    listeners.logStopped({ payload: { sessionId: 42, path: '/logs/web-1.log', bytes: 3, error: null } });
    expect(get(sessionLogs).has(42)).toBe(false);
    expect(get(statusNotice)?.message).toBe('Saved log to /logs/web-1.log');
    statusNotice.set(null);
  });

  it('terminal-exited closes the tab whose backend id matches', async () => {
    await startEventBridge();
    const tab = sessions.spawn('terminal', 'web-1');
    sessions.setTermId(tab.id, 42); // the backend public id the event carries

    listeners.terminalExited({ payload: { sessionId: 42 } });

    expect(get(sessions).some((s) => s.id === tab.id)).toBe(false);
  });

  it('routes sftp events into the matching session by its backend id (§3.4)', async () => {
    await startEventBridge();
    // Two concurrent SFTP tabs; a listing for one must not leak into the other.
    sftp.open(11, 'web-1');
    sftp.open(22, 'db-1');

    listeners.sftpConnected({ payload: { sessionId: 11, hostName: 'web-1' } });
    listeners.sftpDirListed({
      payload: {
        sessionId: 11,
        path: '/srv',
        entries: [{ name: 'app.log', path: '/srv/app.log', size: 12, isDir: false }]
      }
    });

    const tab11 = get(sftp).get(11);
    const tab22 = get(sftp).get(22);
    expect(tab11?.status).toBe('connected');
    expect(tab11?.remote.path).toBe('/srv');
    expect(tab11?.remote.entries).toHaveLength(1);
    // The other tab stays untouched — the stamped session id keeps them apart.
    expect(tab22?.status).toBe('connecting');
    expect(tab22?.remote.entries).toHaveLength(0);

    sftp.remove(11);
    sftp.remove(22);
  });
});
