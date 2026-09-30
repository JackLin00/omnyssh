// Remote-exit routing for split terminal panes (spec C). A pane registers its backend
// session id once `terminal_open` resolves; `terminal-exited` then closes just that
// pane, and the pane's tab decides whether it was the last one.
const handlers = new Map<number, () => void>();

/** Run `onExit` when backend session `backendId` exits. Returns the disposer. */
export function registerPaneExit(backendId: number, onExit: () => void): () => void {
  handlers.set(backendId, onExit);
  return () => {
    if (handlers.get(backendId) === onExit) handlers.delete(backendId);
  };
}

/** Hand an exit to the pane that owns `backendId`; false when no pane does. */
export function dispatchPaneExit(backendId: number): boolean {
  const onExit = handlers.get(backendId);
  if (!onExit) return false;
  handlers.delete(backendId);
  onExit();
  return true;
}
