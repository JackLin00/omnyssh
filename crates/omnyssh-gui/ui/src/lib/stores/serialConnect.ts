import { writable } from 'svelte/store';

// The "open serial port" dialog, opened from the sidebar's Serial entry. A one-flag
// store like `support`: an overlay, never a Content entity.
function createSerialConnect() {
  const { subscribe, set } = writable(false);
  return {
    subscribe,
    open: () => set(true),
    close: () => set(false)
  };
}

export const serialConnect = createSerialConnect();
