// Mouse clipboard for an xterm, PuTTY-style: the selection is copied when a drag ends,
// and a right-click pastes. Each switch is read when the event fires, so flipping it
// in Settings applies to terminals that are already open. Reading goes through the
// clipboard plugin (the webviews' own readText may prompt or be refused); writing uses
// the same navigator.clipboard call as the Ctrl+Shift+C copy.
import { get } from 'svelte/store';
import type { Terminal } from '@xterm/xterm';
import { copyOnSelect, rightClickPaste } from '$lib/stores/terminalPrefs';
import { lastError } from '$lib/stores/notifications';

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

type ClipboardTerm = Pick<Terminal, 'hasSelection' | 'getSelection' | 'paste'>;

/** Wire mouse copy/paste onto `term`, mounted in `container`. `canPaste` is false for
 *  a terminal that sends nothing (the serial monitor). Returns the disposer. */
export function attachMouseClipboard(
  term: ClipboardTerm,
  container: HTMLElement,
  canPaste: boolean
): () => void {
  function onMouseUp(): void {
    if (!get(copyOnSelect) || !term.hasSelection()) return;
    navigator.clipboard.writeText(term.getSelection()).catch((err) => {
      lastError.set(`Copy failed: ${message(err)}`);
    });
  }

  // xterm tracks a selection drag on `document`, not the container — a drag released
  // outside it (e.g. past the top edge to auto-scroll) must still count as a mouseup.
  // Listen on document only while a left-button drag is in progress, one-shot so it
  // never leaks past the release; a second press before release replaces it rather
  // than stacking another listener.
  function onMouseDown(e: MouseEvent): void {
    if (e.button !== 0) return;
    document.removeEventListener('mouseup', onMouseUp);
    document.addEventListener('mouseup', onMouseUp, { once: true });
  }

  function onContextMenu(e: MouseEvent): void {
    if (!canPaste || !get(rightClickPaste)) return; // the native menu stays
    // xterm's own contextmenu handler runs first (it moves its textarea under the
    // pointer, and on macOS selects the word); preventDefault on the ancestor here
    // only suppresses the native menu.
    e.preventDefault();
    void (async () => {
      try {
        const { readText } = await import('@tauri-apps/plugin-clipboard-manager');
        const text = await readText();
        // xterm's own paste path: bracketed-paste mode and line endings as Ctrl+Shift+V.
        if (text) term.paste(text);
      } catch (err) {
        lastError.set(`Paste failed: ${message(err)}`);
      }
    })();
  }

  container.addEventListener('mousedown', onMouseDown);
  container.addEventListener('contextmenu', onContextMenu);
  return () => {
    container.removeEventListener('mousedown', onMouseDown);
    container.removeEventListener('contextmenu', onContextMenu);
    document.removeEventListener('mouseup', onMouseUp);
  };
}
