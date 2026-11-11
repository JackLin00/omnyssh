// Where a file dropped on the window lands (plan Q). Tauri takes file drops from the OS
// before the page sees them and reports them with a position, so the page's own drag
// events never fire; one listener, started at boot, finds the terminal or serial view
// under the pointer. Views register their root element; a hidden tab is `display:
// none`, so only what is on screen can be hit.
import { writable } from 'svelte/store';
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
  function hover(x: number, y: number): { el: Element; target: DropTarget } | null {
    const found = targetFor(hit(x, y));
    dropHover.set(
      found
        ? { el: found.el, label: found.target.label(), protocol: found.target.protocol(), count }
        : null
    );
    return found;
  }
  return (e: DragEvent): void => {
    switch (e.type) {
      case 'enter':
        count = e.paths.length;
        hover(e.x, e.y);
        break;
      case 'over':
        hover(e.x, e.y);
        break;
      case 'drop': {
        dropHover.set(null);
        const found = targetFor(hit(e.x, e.y));
        if (found && e.paths.length > 0) found.target.drop(e.paths);
        break;
      }
      case 'leave':
        dropHover.set(null);
        break;
    }
  };
}

/** Starts following OS file drags over the window. Outside Tauri (a plain browser,
 *  a test page without webview metadata) it does nothing. */
export async function startDropListener(): Promise<() => void> {
  try {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview');
    const track = createDragTracker((x, y) => document.elementFromPoint(x, y));
    const scale = (v: number) => v / (window.devicePixelRatio || 1);
    return await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === 'leave') track({ type: 'leave' });
      else if (p.type === 'over') track({ type: 'over', x: scale(p.position.x), y: scale(p.position.y) });
      else track({ type: p.type, paths: p.paths, x: scale(p.position.x), y: scale(p.position.y) });
    });
  } catch {
    return () => {};
  }
}
