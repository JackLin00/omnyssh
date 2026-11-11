// Where a file dropped on the window lands (plan Q). Tauri takes file drops from the OS
// before the page sees them and reports them with a position, so the page's own drag
// events never fire; one listener, started at boot, finds the terminal or serial view
// under the pointer. Views register their root element; a hidden tab is `display:
// none`, so only what is on screen can be hit.
import { get, writable } from 'svelte/store';
import { isWindows } from '$lib/platform';
import type { UploadProtocol } from '$lib/stores/uploads';

export interface DropTarget {
  /** The session name the overlay names, e.g. `web-1` or `COM3 · 115200`. */
  label: () => string;
  /** The protocol a drop would use now (the tab's override, else its setting). */
  protocol: () => UploadProtocol;
  /** Files were dropped here. */
  drop: (paths: string[]) => void;
}

/** What is being dragged over, for the overlay of the view whose root is `el`. */
export interface DropHover {
  el: Element;
  label: string;
  protocol: UploadProtocol;
  count: number;
}

const targets = new Map<Element, DropTarget>();

export const dropHover = writable<DropHover | null>(null);

export function registerDropTarget(el: Element, target: DropTarget): () => void {
  targets.set(el, target);
  return () => {
    if (targets.get(el) === target) targets.delete(el);
  };
}

/** The registered view `el` is inside, if any. */
export function targetFor(el: Element | null): { el: Element; target: DropTarget } | null {
  for (let node = el; node; node = node.parentElement) {
    const target = targets.get(node);
    if (target) return { el: node, target };
  }
  return null;
}

/** What the OS reports, in the page's terms: Tauri gives physical pixels. */
export type DragEvent =
  | { type: 'enter'; paths: string[]; x: number; y: number }
  | { type: 'over'; x: number; y: number }
  | { type: 'drop'; paths: string[]; x: number; y: number }
  | { type: 'leave' };

/** Follows one drag: `hit` finds the element at a point (`document.elementFromPoint`
 *  in the app). Updates `dropHover` and hands a drop to its target. Pure apart from
 *  the store, so it is testable without a window. */
export function createDragTracker(hit: (x: number, y: number) => Element | null) {
  let count = 0;
  // False for a drag that carries no files (macOS reports text and URL drags too).
  let active = false;
  function hover(x: number, y: number): void {
    const found = targetFor(hit(x, y));
    const next: DropHover | null = found
      ? { el: found.el, label: found.target.label(), protocol: found.target.protocol(), count }
      : null;
    const cur = get(dropHover);
    const same =
      cur === next ||
      (cur !== null &&
        next !== null &&
        cur.el === next.el &&
        cur.label === next.label &&
        cur.protocol === next.protocol &&
        cur.count === next.count);
    if (!same) dropHover.set(next);
  }
  return (e: DragEvent): void => {
    switch (e.type) {
      case 'enter':
        count = e.paths.length;
        active = count > 0;
        if (active) hover(e.x, e.y);
        else dropHover.set(null);
        break;
      case 'over':
        if (active) hover(e.x, e.y);
        break;
      case 'drop': {
        dropHover.set(null);
        if (!active) break;
        active = false;
        const found = targetFor(hit(e.x, e.y));
        if (found && e.paths.length > 0) found.target.drop(e.paths);
        break;
      }
      case 'leave':
        active = false;
        dropHover.set(null);
        break;
    }
  };
}

/** A position Tauri reports, in page (CSS) pixels. Tauri wraps wry's raw position as
 *  physical without converting it, but wry's platforms differ: WebView2 reports
 *  `ScreenToClient` physical pixels, while WKWebView (`draggingLocation`) and WebKitGTK
 *  (widget coordinates) already report logical ones. So only divide when `physical`. */
export function toPagePoint(
  position: { x: number; y: number },
  physical: boolean,
  dpr: number
): { x: number; y: number } {
  return physical ? { x: position.x / dpr, y: position.y / dpr } : { x: position.x, y: position.y };
}

/** Starts following OS file drags over the window. Outside Tauri (a plain browser,
 *  a test page without webview metadata) it does nothing. */
export async function startDropListener(): Promise<() => void> {
  try {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    const track = createDragTracker((x, y) => document.elementFromPoint(x, y));
    const at = (p: { x: number; y: number }) => toPagePoint(p, isWindows, window.devicePixelRatio || 1);
    return await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'leave') track({ type: 'leave' });
      else if (p.type === 'over') track({ type: 'over', ...at(p.position) });
      else track({ type: p.type, paths: p.paths, ...at(p.position) });
    });
  } catch (e) {
    // Outside Tauri this is expected; inside it, setup really failed.
    const w = window as unknown as { __TAURI_INTERNALS__?: { metadata?: unknown } };
    if (w.__TAURI_INTERNALS__?.metadata) console.warn('drag-and-drop upload is unavailable', e);
    return () => {};
  }
}
