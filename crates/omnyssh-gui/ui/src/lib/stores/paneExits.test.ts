import { describe, expect, it, vi } from 'vitest';
import { dispatchPaneExit, registerPaneExit } from './paneExits';

describe('pane exit routing', () => {
  it('runs the handler registered for a backend id, once', () => {
    const onExit = vi.fn();
    registerPaneExit(10, onExit);
    expect(dispatchPaneExit(10)).toBe(true);
    expect(dispatchPaneExit(10)).toBe(false);
    expect(onExit).toHaveBeenCalledTimes(1);
  });

  it('reports an id nobody registered', () => {
    expect(dispatchPaneExit(11)).toBe(false);
  });

  it('the disposer unregisters', () => {
    const onExit = vi.fn();
    const off = registerPaneExit(12, onExit);
    off();
    expect(dispatchPaneExit(12)).toBe(false);
    expect(onExit).not.toHaveBeenCalled();
  });
});
