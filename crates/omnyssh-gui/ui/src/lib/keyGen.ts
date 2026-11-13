// Key generators: user code defining `generateKey(deviceInfo, address)`, which
// returns `{ privateKey, fingerprint, … }` for one device. It runs in a Web Worker
// of its own — `crypto.subtle` and plain JS, none of the app — made fresh for each
// run and stopped after RUN_TIMEOUT_MS. Logins ask for keys with
// `generated-key-required`; the settings page runs a generator to test it.
import type { GeneratedKeyRequired } from '$lib/bindings';
import { answerGeneratedKey } from '$lib/ipc/commands';

/** How long one run may take before its worker is stopped. */
export const RUN_TIMEOUT_MS = 10_000;

export interface KeyRequest {
  code: string;
  /** JSON text; empty for none. */
  credential: string;
  mac: string;
  /** Left out of deviceInfo when absent: the code applies its own default. */
  revision?: number | null;
  /** The host's address, `generateKey`'s second argument. */
  address: string;
}

/** What the app takes from `generateKey`'s result. */
export interface GeneratedKey {
  privateKey: string;
  /** As the code reports it, to compare with the key's real one. */
  fingerprint?: string;
}

/** `generateKey`'s first argument: `{ credential, mac, revision? }`. */
export function deviceInfo(
  credential: string,
  mac: string,
  revision?: number | null
): Record<string, unknown> {
  let parsed: unknown = null;
  if (credential.trim() !== '') {
    try {
      parsed = JSON.parse(credential);
    } catch {
      throw new Error('The credential is not valid JSON');
    }
  }
  const info: Record<string, unknown> = { credential: parsed, mac };
  if (revision != null) info.revision = revision;
  return info;
}

/** The worker's script: the user's code, then a handler that calls it once. */
export function workerSource(code: string): string {
  return `${code}
;self.onmessage = async (e) => {
  try {
    if (typeof generateKey !== 'function') throw new Error('The code does not define generateKey');
    const keys = await generateKey(e.data.deviceInfo, e.data.address);
    self.postMessage({ ok: true, privateKey: keys && keys.privateKey, fingerprint: keys && keys.fingerprint });
  } catch (err) {
    // Something thrown that cannot become a string must still be answered.
    let error = 'The code failed';
    try { error = err instanceof Error ? err.message : String(err); } catch {}
    self.postMessage({ ok: false, error });
  }
};
`;
}

/** The key in a worker's reply, or why there is none. */
export function keyFromReply(reply: unknown): GeneratedKey {
  const r = (reply ?? {}) as { ok?: boolean; error?: unknown; privateKey?: unknown; fingerprint?: unknown };
  if (!r.ok) throw new Error(typeof r.error === 'string' ? r.error : 'The code failed');
  if (typeof r.privateKey !== 'string') throw new Error('The result has no privateKey');
  if (!r.privateKey.trimStart().startsWith('-----BEGIN')) {
    throw new Error('privateKey is not private key text (-----BEGIN …)');
  }
  return {
    privateKey: r.privateKey,
    fingerprint: typeof r.fingerprint === 'string' ? r.fingerprint : undefined
  };
}

/** The part of a Worker the runner uses. */
export interface WorkerLike {
  onmessage: ((e: { data: unknown }) => void) | null;
  onerror: ((e: { message?: string }) => void) | null;
  postMessage(message: unknown): void;
  terminate(): void;
}

/** A real Worker running `source`. */
export function spawnWorker(source: string): WorkerLike {
  const url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
  const worker = new Worker(url);
  const terminate = worker.terminate.bind(worker);
  // The URL must outlive the worker's load; let it go with the worker.
  worker.terminate = () => {
    terminate();
    URL.revokeObjectURL(url);
  };
  return worker as unknown as WorkerLike;
}

/** Runs `request`'s generator once. Rejects with the reason it gave no key. */
export function runGenerator(
  request: KeyRequest,
  { spawn = spawnWorker, timeoutMs = RUN_TIMEOUT_MS }: { spawn?: (source: string) => WorkerLike; timeoutMs?: number } = {}
): Promise<GeneratedKey> {
  let info: Record<string, unknown>;
  try {
    info = deviceInfo(request.credential, request.mac, request.revision);
  } catch (e) {
    return Promise.reject(e);
  }
  return new Promise((resolve, reject) => {
    const worker = spawn(workerSource(request.code));
    const finish = (settle: () => void): void => {
      clearTimeout(timer);
      worker.onmessage = null;
      worker.onerror = null;
      worker.terminate();
      settle();
    };
    const timer = setTimeout(
      () => finish(() => reject(new Error(`The code did not finish within ${timeoutMs / 1000} s`))),
      timeoutMs
    );
    worker.onmessage = (e) =>
      finish(() => {
        try {
          resolve(keyFromReply(e.data));
        } catch (err) {
          reject(err);
        }
      });
    // A syntax error in the code, or one thrown before generateKey is reached.
    worker.onerror = (e) => finish(() => reject(new Error(e.message || 'The code failed to load')));
    worker.postMessage({ deviceInfo: info, address: request.address });
  });
}

/** Answers a login's key request: the key, or why there is none. */
export async function applyGeneratedKeyRequired(
  payload: GeneratedKeyRequired,
  run: (request: KeyRequest) => Promise<GeneratedKey> = runGenerator
): Promise<void> {
  let key: string | null = null;
  let error: string | null = null;
  try {
    key = (await run(payload)).privateKey;
  } catch (e) {
    error = e instanceof Error ? e.message : String(e);
  }
  // The login may have stopped waiting; nothing else is waiting on this.
  await answerGeneratedKey(payload.requestId, key, error).catch(() => {});
}
