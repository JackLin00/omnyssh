// @vitest-environment jsdom
import { describe, it, expect, beforeEach } from 'vitest';
import { get } from 'svelte/store';
import { createDragTracker, dropHover, registerDropTarget, targetFor, toPagePoint, type DropTarget } from './dropTargets';
import type { UploadProtocol } from '$lib/stores/uploads';

function target(protocol: UploadProtocol, dropped: string[][]): DropTarget {
  return { label: () => 'web-1', protocol: () => protocol, drop: (paths) => dropped.push(paths) };
}

describe('drop targets', () => {
  let root: HTMLElement;
  let inner: HTMLElement;
  let outside: HTMLElement;

  beforeEach(() => {
    document.body.innerHTML = '<div id="root"><div><span id="inner"></span></div></div><p id="out"></p>';
    root = document.getElementById('root')!;
    inner = document.getElementById('inner')!;
    outside = document.getElementById('out')!;
    dropHover.set(null);
  });

  it('finds the registered view an element is inside', () => {
    const off = registerDropTarget(root, target('zmodem', []));
    expect(targetFor(inner)?.el).toBe(root);
    expect(targetFor(outside)).toBeNull();
    expect(targetFor(null)).toBeNull();
    off();
    expect(targetFor(inner)).toBeNull();
  });

  it('shows the overlay while dragging over a view and drops there', () => {
    const dropped: string[][] = [];
    const off = registerDropTarget(root, target('sftp', dropped));
    let under: Element | null = inner;
    const track = createDragTracker(() => under);

    track({ type: 'enter', paths: ['/a', '/b'], x: 1, y: 1 });
    expect(get(dropHover)).toEqual({ el: root, label: 'web-1', protocol: 'sftp', count: 2 });
    under = outside;
    track({ type: 'over', x: 2, y: 2 });
    expect(get(dropHover)).toBeNull();
    under = inner;
    track({ type: 'over', x: 1, y: 1 });
    expect(get(dropHover)?.el).toBe(root);
    track({ type: 'drop', paths: ['/a', '/b'], x: 1, y: 1 });
    expect(dropped).toEqual([['/a', '/b']]);
    expect(get(dropHover)).toBeNull();
    off();
  });

  it('ignores a drop outside every view, and a cancelled drag', () => {
    const dropped: string[][] = [];
    const off = registerDropTarget(root, target('zmodem', dropped));
    const track = createDragTracker(() => outside);
    track({ type: 'enter', paths: ['/a'], x: 0, y: 0 });
    track({ type: 'drop', paths: ['/a'], x: 0, y: 0 });
    expect(dropped).toEqual([]);
    const over = createDragTracker(() => inner);
    over({ type: 'enter', paths: ['/a'], x: 0, y: 0 });
    over({ type: 'leave' });
    expect(get(dropHover)).toBeNull();
    expect(dropped).toEqual([]);
    off();
  });

  it('an old registration cannot remove a newer one for the same element', () => {
    const first = registerDropTarget(root, target('zmodem', []));
    registerDropTarget(root, target('xmodem', []));
    first();
    expect(targetFor(root)?.target.protocol()).toBe('xmodem');
  });

  it('a drag with no files shows nothing and drops nothing until a drag with files enters', () => {
    const dropped: string[][] = [];
    const off = registerDropTarget(root, target('zmodem', dropped));
    const track = createDragTracker(() => inner);
    track({ type: 'enter', paths: [], x: 1, y: 1 });
    track({ type: 'over', x: 1, y: 1 });
    expect(get(dropHover)).toBeNull();
    track({ type: 'drop', paths: [], x: 1, y: 1 });
    expect(dropped).toEqual([]);
    track({ type: 'enter', paths: ['/a'], x: 1, y: 1 });
    expect(get(dropHover)?.count).toBe(1);
    track({ type: 'leave' });
    track({ type: 'over', x: 1, y: 1 });
    expect(get(dropHover)).toBeNull();
    off();
  });

  it('does not rewrite the hover when nothing changed', () => {
    const off = registerDropTarget(root, target('zmodem', []));
    const track = createDragTracker(() => inner);
    track({ type: 'enter', paths: ['/a'], x: 1, y: 1 });
    let calls = 0;
    const unsub = dropHover.subscribe(() => calls++);
    calls = 0;
    track({ type: 'over', x: 2, y: 2 });
    track({ type: 'over', x: 3, y: 3 });
    expect(calls).toBe(0);
    unsub();
    off();
  });
});

describe('toPagePoint', () => {
  it('divides physical pixels by the device pixel ratio', () => {
    expect(toPagePoint({ x: 200, y: 100 }, true, 2)).toEqual({ x: 100, y: 50 });
  });
  it('leaves already-logical positions alone', () => {
    expect(toPagePoint({ x: 200, y: 100 }, false, 2)).toEqual({ x: 200, y: 100 });
  });
});
