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
  function onMouseUp(e: MouseEvent): void {
    if (e.button !== 0 || !get(copyOnSelect) || !term.hasSelection()) return;
    navigator.clipboard.writeText(term.getSelection()).catch((err) => {
      lastError.set(`Copy failed: ${message(err)}`);
    });
  }

  function onContextMenu(e: MouseEvent): void {
    if (!canPaste || !get(rightClickPaste)) return; // the native menu stays
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

  container.addEventListener('mouseup', onMouseUp);
  container.addEventListener('contextmenu', onContextMenu);
  return () => {
    container.removeEventListener('mouseup', onMouseUp);
    container.removeEventListener('contextmenu', onContextMenu);
  };
}
