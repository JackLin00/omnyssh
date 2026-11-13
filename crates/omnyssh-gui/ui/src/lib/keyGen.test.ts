import { beforeEach, describe, expect, it, vi } from 'vitest';

const { answered } = vi.hoisted(() => ({
  answered: [] as Array<[number, string | null, string | null]>
}));
vi.mock('$lib/ipc/commands', () => ({
  answerGeneratedKey: (id: number, key: string | null, error: string | null) => {
    answered.push([id, key, error]);
    return Promise.resolve();
  }
}));

import {
  applyGeneratedKeyRequired,
  deviceInfo,
  keyFromReply,
  runGenerator,
  workerSource,
  type WorkerLike
} from './keyGen';

/** Runs a worker script here, against a stand-in `self`: what a Worker would do. */
function inProcess(source: string): WorkerLike & { terminated: boolean } {
  const worker = {
    onmessage: null as WorkerLike['onmessage'],
    onerror: null as WorkerLike['onerror'],
    terminated: false,
    postMessage(data: unknown) {
      queueMicrotask(() => scope.onmessage?.({ data }));
    },
    terminate() {
      worker.terminated = true;
    }
  };
  const scope: { onmessage?: (e: { data: unknown }) => void; postMessage: (data: unknown) => void } = {
    postMessage: (data) => {
      if (!worker.terminated) worker.onmessage?.({ data });
    }
  };
  try {
    new Function('self', source)(scope);
  } catch (e) {
    queueMicrotask(() => worker.onerror?.({ message: (e as Error).message }));
  }
  return worker;
}

const KEY = '-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----\n';

/** Echoes what it was given into the key text, hashing the MAC with Web Crypto. */
const ECHO = `
async function generateKey(deviceInfo, address) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(deviceInfo.mac));
  const hex = Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, '0')).join('');
  return {
    privateKey: '-----BEGIN OPENSSH PRIVATE KEY-----\\n' + JSON.stringify({ deviceInfo, address, hex }) + '\\n',
    publicKey: 'ssh-ed25519 AAAA',
    fingerprint: 'SHA256:abc'
  };
}`;

const request = (code: string, over: Record<string, unknown> = {}) => ({
  code,
  credential: '{"master_key":"00ff"}',
  mac: '30:ae:7b:eb:17:82',
  revision: 2,
  address: '192.168.11.1',
  ...over
});

describe('deviceInfo', () => {
  it('parses the credential and leaves revision out when there is none', () => {
    expect(deviceInfo('{"master_key":"00ff"}', 'aa', 2)).toEqual({
      credential: { master_key: '00ff' },
      mac: 'aa',
      revision: 2
    });
    expect(deviceInfo('', 'aa', null)).toEqual({ credential: null, mac: 'aa' });
    expect('revision' in deviceInfo('', 'aa')).toBe(false);
  });

  it('refuses a credential that is not JSON', () => {
    expect(() => deviceInfo('{master_key: 1}', 'aa')).toThrow('The credential is not valid JSON');
  });
});

describe('keyFromReply', () => {
  it('takes the private key and the reported fingerprint', () => {
    expect(keyFromReply({ ok: true, privateKey: KEY, fingerprint: 'SHA256:x' })).toEqual({
      privateKey: KEY,
      fingerprint: 'SHA256:x'
    });
  });

  it('says why there is no key', () => {
    expect(() => keyFromReply({ ok: false, error: 'boom' })).toThrow('boom');
    expect(() => keyFromReply({ ok: true })).toThrow('The result has no privateKey');
    expect(() => keyFromReply({ ok: true, privateKey: 'ssh-ed25519 AAAA' })).toThrow('not private key text');
  });
});

describe('runGenerator', () => {
  it('calls generateKey with the device info and the address, Web Crypto included', async () => {
    const key = await runGenerator(request(ECHO), { spawn: inProcess });
    expect(key.fingerprint).toBe('SHA256:abc');
    const echoed = JSON.parse(key.privateKey.split('\n')[1]);
    expect(echoed.deviceInfo).toEqual({ credential: { master_key: '00ff' }, mac: '30:ae:7b:eb:17:82', revision: 2 });
    expect(echoed.address).toBe('192.168.11.1');
    expect(echoed.hex).toMatch(/^[0-9a-f]{64}$/);
  });

  it('reports what the code threw, and code without generateKey', async () => {
    const thrown = 'async function generateKey() { throw new Error("master_key is undefined"); }';
    await expect(runGenerator(request(thrown), { spawn: inProcess })).rejects.toThrow('master_key is undefined');
    await expect(runGenerator(request('const x = 1;'), { spawn: inProcess })).rejects.toThrow(
      'The code does not define generateKey'
    );
    await expect(runGenerator(request('function ('), { spawn: inProcess })).rejects.toThrow();
  });

  it('refuses a credential that is not JSON before running anything', async () => {
    const spawn = vi.fn(inProcess);
    await expect(runGenerator(request(ECHO, { credential: 'nope' }), { spawn })).rejects.toThrow('not valid JSON');
    expect(spawn).not.toHaveBeenCalled();
  });

  it('stops code that runs too long, and every worker when it is done', async () => {
    const workers: Array<{ terminated: boolean }> = [];
    const spawn = (source: string) => {
      const w = inProcess(source);
      workers.push(w);
      return w;
    };
    const hangs = 'async function generateKey() { await new Promise(() => {}); }';
    await expect(runGenerator(request(hangs), { spawn, timeoutMs: 50 })).rejects.toThrow('did not finish within');
    await runGenerator(request(ECHO), { spawn });
    expect(workers.map((w) => w.terminated)).toEqual([true, true]);
  });
});

describe('applyGeneratedKeyRequired', () => {
  beforeEach(() => answered.splice(0));

  const payload = { requestId: 7, hostName: 'cam', ...request(ECHO) };

  it('answers with the key', async () => {
    await applyGeneratedKeyRequired(payload, () => Promise.resolve({ privateKey: KEY }));
    expect(answered).toEqual([[7, KEY, null]]);
  });

  it('answers with why there is none', async () => {
    await applyGeneratedKeyRequired(payload, () => Promise.reject(new Error('boom')));
    expect(answered).toEqual([[7, null, 'boom']]);
  });
});
