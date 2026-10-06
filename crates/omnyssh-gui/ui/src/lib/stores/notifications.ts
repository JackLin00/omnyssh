import { writable } from 'svelte/store';

/** How long a surfaced error stays in the status bar before it clears itself (ms). */
export const ERROR_TTL_MS = 8000;

/** A status-bar message that clears itself after `ERROR_TTL_MS`, so the bar returns to
 *  its usual content instead of pinning a stale one for the whole session; a fresh
 *  message restarts the window, `set(null)` clears now. */
function createTimed<T>() {
  const { subscribe, set } = writable<T | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  return {
    subscribe,
    set(message: T | null): void {
      if (timer !== undefined) clearTimeout(timer);
      timer = undefined;
      set(message);
      if (message) {
        timer = setTimeout(() => {
          timer = undefined;
          set(null);
        }, ERROR_TTL_MS);
        // Node's timers keep the test event loop alive; don't hold it open (no-op in the
        // webview, whose timers have no `unref`).
        (timer as { unref?: () => void }).unref?.();
      }
    }
  };
}

/** Last error surfaced to the status bar (tech-gui.md §3.5, §4.2). */
export const lastError = createTimed<string>();

/** A short confirmation in the status bar, e.g. that a session log was saved. */
export interface StatusNotice {
  message: string;
  /** A saved file the notice offers to show in its folder. */
  path?: string;
}

/** The latest notice; an error, shown in its place while there is one, wins. */
export const statusNotice = createTimed<StatusNotice>();
