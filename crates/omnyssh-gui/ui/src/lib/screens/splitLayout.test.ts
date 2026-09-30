import { describe, expect, it } from 'vitest';
import {
  arrange,
  clampRatio,
  neighbor,
  panes,
  remove,
  setRatio,
  split,
  type Layout
} from './splitLayout';

const pane = (id: number): Layout => ({ kind: 'pane', id });

describe('split', () => {
  it('turns the target pane into a half-and-half split with the new pane second', () => {
    expect(split(pane(1), 1, 'row', 2)).toEqual({
      kind: 'split',
      dir: 'row',
      ratio: 0.5,
      a: pane(1),
      b: pane(2)
    });
  });

  it('nests inside an existing split and leaves other panes alone', () => {
    const two = split(pane(1), 1, 'row', 2);
    const three = split(two, 2, 'column', 3);
    expect(panes(three)).toEqual([1, 2, 3]);
    expect(three).toMatchObject({ a: pane(1), b: { kind: 'split', dir: 'column', a: pane(2), b: pane(3) } });
  });

  it('returns the same tree for an unknown target', () => {
    const two = split(pane(1), 1, 'row', 2);
    expect(split(two, 9, 'row', 3)).toBe(two);
  });
});

describe('remove', () => {
  it('lets the sibling take the parent split\'s place', () => {
    const three = split(split(pane(1), 1, 'row', 2), 2, 'column', 3);
    expect(remove(three, 3)).toEqual({ kind: 'split', dir: 'row', ratio: 0.5, a: pane(1), b: pane(2) });
    expect(remove(three, 1)).toEqual({ kind: 'split', dir: 'column', ratio: 0.5, a: pane(2), b: pane(3) });
  });

  it('returns null when the last pane goes', () => {
    expect(remove(pane(1), 1)).toBeNull();
  });
});

describe('neighbor', () => {
  it('is the first pane of the closing pane\'s sibling', () => {
    const three = split(split(pane(1), 1, 'row', 2), 2, 'column', 3);
    expect(neighbor(three, 1)).toBe(2);
    expect(neighbor(three, 3)).toBe(2);
    expect(neighbor(three, 2)).toBe(3);
    expect(neighbor(pane(1), 1)).toBeNull();
  });
});

describe('setRatio', () => {
  it('changes only the split at the path', () => {
    const three = split(split(pane(1), 1, 'row', 2), 2, 'column', 3);
    const moved = setRatio(three, ['b'], 0.25);
    expect(moved).toMatchObject({ ratio: 0.5, b: { ratio: 0.25 } });
    expect(setRatio(three, [], 0.75)).toMatchObject({ ratio: 0.75, b: { ratio: 0.5 } });
  });
});

describe('clampRatio', () => {
  it('keeps each side at least the minimum size', () => {
    // 1000px shared, 120px minimum: 0.12..0.88.
    expect(clampRatio(0.05, 1000, 120)).toBeCloseTo(0.12);
    expect(clampRatio(0.95, 1000, 120)).toBeCloseTo(0.88);
    expect(clampRatio(0.4, 1000, 120)).toBe(0.4);
  });

  it('never lets a side drop under a tenth, and centres when there is no room', () => {
    expect(clampRatio(0.01, 10000, 120)).toBe(0.1);
    expect(clampRatio(0.9, 200, 120)).toBe(0.5);
    expect(clampRatio(0.3, 0, 120)).toBe(0.5);
  });
});

describe('arrange', () => {
  it('gives a single pane the whole tab and draws no divider', () => {
    expect(arrange(pane(1))).toEqual({ panes: [{ id: 1, rect: { x: 0, y: 0, w: 1, h: 1 } }], dividers: [] });
  });

  it('lays nested splits out as flat rectangles with a divider per split', () => {
    const layout = split(split(pane(1), 1, 'row', 2), 2, 'column', 3);
    const { panes: rects, dividers } = arrange(layout);
    expect(rects).toEqual([
      { id: 1, rect: { x: 0, y: 0, w: 0.5, h: 1 } },
      { id: 2, rect: { x: 0.5, y: 0, w: 0.5, h: 0.5 } },
      { id: 3, rect: { x: 0.5, y: 0.5, w: 0.5, h: 0.5 } }
    ]);
    expect(dividers).toEqual([
      { path: [], dir: 'row', rect: { x: 0.5, y: 0, w: 0, h: 1 }, parent: { x: 0, y: 0, w: 1, h: 1 } },
      {
        path: ['b'],
        dir: 'column',
        rect: { x: 0.5, y: 0.5, w: 0.5, h: 0 },
        parent: { x: 0.5, y: 0, w: 0.5, h: 1 }
      }
    ]);
  });
});
