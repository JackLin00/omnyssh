// The split tree of a terminal tab: panes nested in side-by-side (`row`) and stacked
// (`column`) splits, each split sharing its space by `ratio`. Pure and immutable. The
// view renders it through `arrange`, which flattens the tree into rectangles, so panes
// are positioned absolutely and never remount when the tree changes shape.

export type PaneId = number;
/** `row` puts the two halves side by side; `column` stacks them. */
export type SplitDir = 'row' | 'column';
export type Layout =
  | { kind: 'pane'; id: PaneId }
  | { kind: 'split'; dir: SplitDir; ratio: number; a: Layout; b: Layout };
/** The way from the root to a split: `a` or `b` at each level. */
export type Path = ('a' | 'b')[];
/** A rectangle in fractions of the tab (0..1). */
export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface PaneRect {
  id: PaneId;
  rect: Rect;
}
/** One split's boundary: `rect` is a zero-width (row) or zero-height (column) line,
 *  `parent` the space the split shares, which a drag measures the ratio against. */
export interface DividerRect {
  path: Path;
  dir: SplitDir;
  rect: Rect;
  parent: Rect;
}

/** Split pane `target` in two along `dir`; the new pane `created` takes the second half. */
export function split(layout: Layout, target: PaneId, dir: SplitDir, created: PaneId): Layout {
  if (layout.kind === 'pane') {
    if (layout.id !== target) return layout;
    return { kind: 'split', dir, ratio: 0.5, a: layout, b: { kind: 'pane', id: created } };
  }
  const a = split(layout.a, target, dir, created);
  const b = split(layout.b, target, dir, created);
  return a === layout.a && b === layout.b ? layout : { ...layout, a, b };
}

/** Drop pane `target`; its sibling takes the parent split's place. Null once no pane is left. */
export function remove(layout: Layout, target: PaneId): Layout | null {
  if (layout.kind === 'pane') return layout.id === target ? null : layout;
  const a = remove(layout.a, target);
  const b = remove(layout.b, target);
  if (a === null) return b;
  if (b === null) return a;
  return a === layout.a && b === layout.b ? layout : { ...layout, a, b };
}

/** Every pane, left to right and top to bottom. */
export function panes(layout: Layout): PaneId[] {
  return layout.kind === 'pane' ? [layout.id] : [...panes(layout.a), ...panes(layout.b)];
}

/** The pane to focus once `target` closes: the first pane of its sibling. */
export function neighbor(layout: Layout, target: PaneId): PaneId | null {
  if (layout.kind === 'pane') return null;
  if (layout.a.kind === 'pane' && layout.a.id === target) return panes(layout.b)[0];
  if (layout.b.kind === 'pane' && layout.b.id === target) return panes(layout.a)[0];
  return neighbor(layout.a, target) ?? neighbor(layout.b, target);
}

/** Set the ratio of the split at `path`. A path that no longer leads to a split is a no-op. */
export function setRatio(layout: Layout, path: Path, ratio: number): Layout {
  if (layout.kind === 'pane') return layout;
  if (path.length === 0) return { ...layout, ratio };
  const [step, ...rest] = path;
  return step === 'a'
    ? { ...layout, a: setRatio(layout.a, rest, ratio) }
    : { ...layout, b: setRatio(layout.b, rest, ratio) };
}

/** Keep both sides of a split of `parentPx` at least `minPx` and a tenth of it; a split
 *  too small for both sits at the middle. */
export function clampRatio(ratio: number, parentPx: number, minPx: number): number {
  const lo = parentPx > 0 ? Math.min(0.5, Math.max(0.1, minPx / parentPx)) : 0.5;
  return Math.min(1 - lo, Math.max(lo, ratio));
}

/** Flatten the tree into pane and divider rectangles within `rect`. */
export function arrange(
  layout: Layout,
  rect: Rect = { x: 0, y: 0, w: 1, h: 1 },
  path: Path = []
): { panes: PaneRect[]; dividers: DividerRect[] } {
  if (layout.kind === 'pane') return { panes: [{ id: layout.id, rect }], dividers: [] };
  const row = layout.dir === 'row';
  const aRect = row ? { ...rect, w: rect.w * layout.ratio } : { ...rect, h: rect.h * layout.ratio };
  const bRect = row
    ? { ...rect, x: rect.x + aRect.w, w: rect.w - aRect.w }
    : { ...rect, y: rect.y + aRect.h, h: rect.h - aRect.h };
  const line = row
    ? { x: bRect.x, y: rect.y, w: 0, h: rect.h }
    : { x: rect.x, y: bRect.y, w: rect.w, h: 0 };
  const a = arrange(layout.a, aRect, [...path, 'a']);
  const b = arrange(layout.b, bRect, [...path, 'b']);
  return {
    panes: [...a.panes, ...b.panes],
    dividers: [{ path, dir: layout.dir, rect: line, parent: rect }, ...a.dividers, ...b.dividers]
  };
}
