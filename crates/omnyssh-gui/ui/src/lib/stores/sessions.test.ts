import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { sessions, sessionLabel, sessionTitle, sessionStatusDot, combineStatus } from './sessions';

// The sessions list backs the sidebar rows and the terminal layer (tech-gui.md §2,
// §3.1). Ids are monotonic (never reused) and each session carries the backend
// `termId` once `terminal_open` resolves; status drives the row dot.
describe('sessions store', () => {
  it('spawns with connecting status and a fresh monotonic id', () => {
    const a = sessions.spawn('terminal', 'web-1');
    const b = sessions.spawn('sftp', 'db-1');
    expect(b.id).toBeGreaterThan(a.id);
    expect(a.status).toBe('connecting');
    expect(a.termId).toBeUndefined();
    sessions.close(a.id);
    sessions.close(b.id);
  });

  it('records the backend termId and updates status without touching other rows', () => {
    const a = sessions.spawn('terminal', 'web-1');
    const b = sessions.spawn('terminal', 'db-1');

    sessions.setTermId(a.id, 99);
    sessions.setStatus(a.id, 'connected');

    const list = get(sessions);
    const rowA = list.find((s) => s.id === a.id);
    const rowB = list.find((s) => s.id === b.id);
    expect(rowA).toMatchObject({ termId: 99, status: 'connected' });
    // Sibling untouched — a per-id update must not bleed across rows.
    expect(rowB?.termId).toBeUndefined();
    expect(rowB?.status).toBe('connecting');

    sessions.close(a.id);
    sessions.close(b.id);
  });

  it('closing removes only the target', () => {
    const a = sessions.spawn('terminal', 'web-1');
    const b = sessions.spawn('terminal', 'db-1');
    sessions.close(a.id);
    const ids = get(sessions).map((s) => s.id);
    expect(ids).toContain(b.id);
    expect(ids).not.toContain(a.id);
    sessions.close(b.id);
  });

  it('shows the host name alone but keeps the type in the accessible title', () => {
    const s = sessions.spawn('terminal', 'web-1');
    // Visible label drops the type (the row icon carries it); the title retains it.
    expect(sessionLabel(s)).toBe('web-1');
    expect(sessionTitle(s)).toBe('web-1 · terminal');
    expect(sessionStatusDot.connected).toBe('ok');
    expect(sessionStatusDot.failed).toBe('crit');
    expect(sessionStatusDot.connecting).toBe('unknown');
    sessions.close(s.id);
  });

  it('a serial session carries its port options', () => {
    const serial = {
      config: {
        port: 'COM3',
        baudRate: 115200,
        dataBits: 8,
        parity: 'none' as const,
        stopBits: 'one' as const,
        flowControl: 'none' as const
      },
      mode: 'monitor' as const,
      enter: 'cr' as const
    };
    const s = sessions.spawn('serial', 'COM3 · 115200', serial);
    expect(get(sessions).find((r) => r.id === s.id)).toMatchObject({
      kind: 'serial',
      hostName: 'COM3 · 115200',
      serial
    });
    sessions.close(s.id);
  });
});

describe('combineStatus', () => {
  it('is connected while any pane is', () => {
    expect(combineStatus(['failed', 'connected', 'connecting'])).toBe('connected');
  });

  it('is failed only once every pane has failed', () => {
    expect(combineStatus(['failed', 'failed'])).toBe('failed');
    expect(combineStatus(['failed', 'connecting'])).toBe('connecting');
  });

  it('is connecting with no pane reported yet', () => {
    expect(combineStatus([])).toBe('connecting');
  });
});
