// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { attachWheelZoom } from './terminalZoom';

function wheel(opts: { deltaY: number; ctrlKey?: boolean; deltaMode?: number }): WheelEvent {
  return new WheelEvent('wheel', {
    deltaY: opts.deltaY,
    ctrlKey: opts.ctrlKey ?? false,
    deltaMode: opts.deltaMode ?? 0,
    cancelable: true
  });
}

describe('attachWheelZoom', () => {
  it('a mouse-wheel notch (±100px) with Ctrl held steps twice, and swallows the event', () => {
    const el = document.createElement('div');
    const onStep = vi.fn();
    attachWheelZoom(el, onStep);

    const e = wheel({ deltaY: -100, ctrlKey: true });
    el.dispatchEvent(e);

    expect(onStep).toHaveBeenCalledTimes(2);
    expect(onStep).toHaveBeenNthCalledWith(1, 1);
    expect(onStep).toHaveBeenNthCalledWith(2, 1);
    expect(e.defaultPrevented).toBe(true);
  });

  it('accumulates small deltas before stepping', () => {
    const el = document.createElement('div');
    const onStep = vi.fn();
    attachWheelZoom(el, onStep);

    el.dispatchEvent(wheel({ deltaY: 30, ctrlKey: true }));
    expect(onStep).not.toHaveBeenCalled();

    el.dispatchEvent(wheel({ deltaY: 30, ctrlKey: true }));
    expect(onStep).toHaveBeenCalledTimes(1);
    expect(onStep).toHaveBeenCalledWith(-1);
  });

  it('does nothing, and does not preventDefault, without Ctrl held', () => {
    const el = document.createElement('div');
    const onStep = vi.fn();
    attachWheelZoom(el, onStep);

    const e = wheel({ deltaY: -100, ctrlKey: false });
    el.dispatchEvent(e);

    expect(onStep).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it('scales LINE-mode deltas (×16) before accumulating', () => {
    const el = document.createElement('div');
    const onStep = vi.fn();
    attachWheelZoom(el, onStep);

    // deltaMode 1 = DOM_DELTA_LINE; -3 lines * 16px = -48px, below the 50px step.
    el.dispatchEvent(wheel({ deltaY: -3, ctrlKey: true, deltaMode: 1 }));
    expect(onStep).not.toHaveBeenCalled();

    // A second -3 lines brings the total to -96px, past the 50px step once.
    el.dispatchEvent(wheel({ deltaY: -3, ctrlKey: true, deltaMode: 1 }));
    expect(onStep).toHaveBeenCalledTimes(1);
    expect(onStep).toHaveBeenCalledWith(1);
  });

  it('stops reacting once disposed', () => {
    const el = document.createElement('div');
    const onStep = vi.fn();
    const dispose = attachWheelZoom(el, onStep);
    dispose();

    el.dispatchEvent(wheel({ deltaY: -100, ctrlKey: true }));
    expect(onStep).not.toHaveBeenCalled();
  });
});
