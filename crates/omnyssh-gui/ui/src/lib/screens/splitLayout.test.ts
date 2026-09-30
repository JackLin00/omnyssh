import { describe, expect, it } from 'vitest';
import {
  arrange,
  clampRatio,
  dropPreview,
  dropSide,
  movePane,
  neighbor,
  paneAt,
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

describe('movePane', () => {
  // 1 | (2 / 3)
  const three = split(split(pane(1), 1, 'row', 2), 2, 'column', 3);

  it('puts the pane on the chosen side of the target, sharing it half and half', () => {
    expect(movePane(three, 1, 3, 'top')).toEqual({
      kind: 'split',
      dir: 'column',
      ratio: 0.5,
      a: pane(2),
      b: { kind: 'split', dir: 'column', ratio: 0.5, a: pane(1), b: pane(3) }
    });
    expect(movePane(three, 3, 1, 'left')).toEqual({
      kind: 'split',
      dir: 'row',
      ratio: 0.5,
      a: { kind: 'split', dir: 'row', ratio: 0.5, a: pane(3), b: pane(1) },
      b: pane(2)
    });
    expect(movePane(three, 2, 1, 'bottom')).toMatchObject({
      a: { kind: 'split', dir: 'column', a: pane(1), b: pane(2) },
      b: pane(3)
    });
  });

  it('works between the two panes of a single split', () => {
    const two = split(pane(1), 1, 'row', 2);
    expect(movePane(two, 1, 2, 'bottom')).toEqual({
      kind: 'split',
      dir: 'column',
      ratio: 0.5,
      a: pane(2),
      b: pane(1)
    });
  });

  it('leaves the layout alone for itself or an unknown pane', () => {
    expect(movePane(three, 1, 1, 'top')).toBe(three);
    expect(movePane(three, 9, 1, 'top')).toBe(three);
    expect(movePane(three, 1, 9, 'top')).toBe(three);
  });

  it('keeps every pane', () => {
    expect(panes(movePane(three, 1, 3, 'right')).sort()).toEqual([1, 2, 3]);
  });
});

describe('dropSide', () => {
  const rect = { x: 0.5, y: 0, w: 0.5, h: 1 };

  it('is the edge nearest the pointer, measured relative to the pane', () => {
    expect(dropSide(0.52, 0.5, rect)).toBe('left');
    expect(dropSide(0.98, 0.5, rect)).toBe('right');
    expect(dropSide(0.75, 0.05, rect)).toBe('top');
    expect(dropSide(0.75, 0.95, rect)).toBe('bottom');
  });
});

describe('dropPreview', () => {
  it('is the half of the target on that side', () => {
    const rect = { x: 0.5, y: 0, w: 0.5, h: 1 };
    expect(dropPreview(rect, 'left')).toEqual({ x: 0.5, y: 0, w: 0.25, h: 1 });
    expect(dropPreview(rect, 'right')).toEqual({ x: 0.75, y: 0, w: 0.25, h: 1 });
    expect(dropPreview(rect, 'top')).toEqual({ x: 0.5, y: 0, w: 0.5, h: 0.5 });
    expect(dropPreview(rect, 'bottom')).toEqual({ x: 0.5, y: 0.5, w: 0.5, h: 0.5 });
  });
});

describe('paneAt', () => {
  it('finds the pane under a point', () => {
    const { panes: rects } = arrange(split(pane(1), 1, 'row', 2));
    expect(paneAt(0.25, 0.5, rects)?.id).toBe(1);
    expect(paneAt(0.75, 0.5, rects)?.id).toBe(2);
    expect(paneAt(1.5, 0.5, rects)).toBeUndefined();
  });
});
