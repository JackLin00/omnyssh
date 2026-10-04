import { derived, writable } from 'svelte/store';
import type { HostKeyChanged } from '$lib/bindings';
import { displayHostname } from './streamer';

// Connections waiting on a changed host key, oldest first. The dialog shows the
// first; each is its own connection, so none is folded into another.
export const hostKeyQueue = writable<HostKeyChanged[]>([]);

export const hostKeyPrompt = derived(hostKeyQueue, (queue) => queue[0] ?? null);

/** Drop the question `requestId`: answered, or no longer waited on. */
export function settleHostKey(requestId: number): void {
  hostKeyQueue.update((queue) => queue.filter((q) => q.requestId !== requestId));
}

/** `Host key changed: host[:port]`, naming the host a bastion was on the way to. */
export function hostKeyTitle(q: Pick<HostKeyChanged, 'host' | 'port' | 'jumpFor'>, streamerOn: boolean): string {
  const where = displayHostname(q.host, streamerOn) + (q.port === 22 ? '' : `:${q.port}`);
  const via = q.jumpFor ? ` (jump host for ${q.jumpFor})` : '';
  return `Host key changed: ${where}${via}`;
}

/** Where a server keeps its public host key of `keyType`, for the check it suggests. */
export function serverKeyFile(keyType: string): string {
  const kind = keyType === 'ssh-rsa' ? 'rsa' : keyType.startsWith('ecdsa-') ? 'ecdsa' : 'ed25519';
  return `/etc/ssh/ssh_host_${kind}_key.pub`;
}
