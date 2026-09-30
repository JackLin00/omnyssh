import { writable } from 'svelte/store';
import type { SerialDeviceDto } from '$lib/bindings';
import { listSerialDevices } from '$lib/ipc/commands';
import { lastError } from './notifications';

// The saved serial devices (serial.toml), shown as dashboard cards. Reloaded after
// every save or delete; a failed load surfaces in the status bar and keeps the list.
export const serialDevices = writable<SerialDeviceDto[]>([]);

export async function reloadSerialDevices(): Promise<void> {
  try {
    serialDevices.set(await listSerialDevices());
  } catch (e) {
    lastError.set(e instanceof Error ? e.message : String(e));
  }
}
