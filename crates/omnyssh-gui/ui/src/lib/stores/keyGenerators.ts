import { writable } from 'svelte/store';
import type { KeyGeneratorsDto } from '$lib/bindings';
import { listKeyGenerators } from '$lib/ipc/commands';
import { lastError } from './notifications';

/** The key generators' switch and the generators (`key_generators.toml`); null until
 *  first loaded. Read by the settings page and the host form. */
export const keyGenerators = writable<KeyGeneratorsDto | null>(null);

/** Reads them from the backend; a failure is reported and leaves the last copy. */
export async function loadKeyGenerators(): Promise<void> {
  try {
    keyGenerators.set(await listKeyGenerators());
  } catch (e) {
    lastError.set(`Key generators: ${e instanceof Error ? e.message : String(e)}`);
  }
}
