<script lang="ts">
  // A terminal tab (tech-gui.md §3.1): one or more panes, each its own SSH session to
  // the tab's host, laid out by a split tree the user can nest and resize (spec C).
  // Panes are positioned absolutely from the tree and keyed by id, so a split or a close
  // never remounts a pane — its xterm and session live on. Kept mounted for the tab's
  // whole life, hidden when another entity is active.
  import { onDestroy, untrack } from 'svelte';
  import { closeSession } from '$lib/stores/navigation';
  import { sessions, combineStatus, type Session, type SessionStatus } from '$lib/stores/sessions';
  import TerminalPane from './TerminalPane.svelte';
  import {
    arrange,
    clampRatio,
    dropPreview,
    dropSide,
    movePane,
    neighbor,
    paneAt,
    remove,
    setRatio,
    split,
    type DividerRect,
    type DropSide,
    type Layout,
    type PaneId,
    type SplitDir
  } from './splitLayout';

  let { session, active }: { session: Session; active: boolean } = $props();

  /** Each pane keeps at least this much room when a divider is dragged. */
  const MIN_PANE_PX = 120;

  let nextPane = 2;
  let layout = $state<Layout>({ kind: 'pane', id: 1 });
  let focused = $state<PaneId>(1);
  let statuses = $state<Record<PaneId, SessionStatus>>({});
  let area: HTMLDivElement;
  const arranged = $derived(arrange(layout));

  // The tab's dot in the sidebar sums up its panes. `setStatus` hands this tab a new
  // session object, so the id is read untracked: tracking it would re-run this forever.
  const tabStatus = $derived(combineStatus(Object.values(statuses)));
  $effect(() => {
    const status = tabStatus;
    untrack(() => sessions.setStatus(session.id, status));
  });

  function splitPane(id: PaneId, dir: SplitDir): void {
    const created = nextPane++;
    layout = split(layout, id, dir, created);
    focused = created;
  }

  // Closing the last pane closes the tab, which unmounts every pane with it.
  function closePane(id: PaneId): void {
    const next = neighbor(layout, id);
    const rest = remove(layout, id);
    delete statuses[id];
    if (rest === null) {
      closeSession(session.id);
      return;
    }
    layout = rest;
    if (focused === id && next !== null) focused = next;
  }

  // Dragging a divider sets its split's ratio from the pointer, measured against the
  // space that split shares, with each side kept at MIN_PANE_PX.
  function startDrag(e: PointerEvent, d: DividerRect): void {
    e.preventDefault();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const row = d.dir === 'row';
    function move(ev: PointerEvent): void {
      const box = area.getBoundingClientRect();
      // The tab was hidden mid-drag: there is nothing to measure against.
      if (box.width === 0 || box.height === 0) return;
      const pos = row ? (ev.clientX - box.left) / box.width : (ev.clientY - box.top) / box.height;
      const start = row ? d.parent.x : d.parent.y;
      const size = row ? d.parent.w : d.parent.h;
      const parentPx = size * (row ? box.width : box.height);
      layout = setRatio(layout, d.path, clampRatio((pos - start) / size, parentPx, MIN_PANE_PX));
    }
    function end(): void {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('pointercancel', end);
      handle.removeEventListener('lostpointercapture', end);
    }
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
    handle.addEventListener('lostpointercapture', end);
  }

  /** How far the pointer must travel before a press on a title bar becomes a move. */
  const MOVE_THRESHOLD_PX = 5;

  let moving = $state<{ id: PaneId; target: PaneId | null; side: DropSide | null } | null>(null);
  const preview = $derived.by(() => {
    if (!moving || moving.target === null || moving.side === null) return null;
    const target = arranged.panes.find((p) => p.id === moving!.target);
    return target ? dropPreview(target.rect, moving.side) : null;
  });
  // Ends the drag in progress, if any — set while one runs, cleared once it ends. A
  // pane can be destroyed mid-drag (remote exit, its tab closing): the handle that held
  // pointer capture is then removed from the DOM, so `lostpointercapture` fires on the
  // document instead of reaching the handle's own listener below, and `end` would never
  // run on its own. The $effect after this function, and `onDestroy`, call it directly.
  let cancelMove: (() => void) | null = null;

  // Moving a pane by its title bar: past a few pixels the press becomes a drag, the pane
  // under the pointer shows where it would land, and a release there moves it. Esc, a
  // release elsewhere, or a lost capture leaves the layout as it was.
  function startMove(e: PointerEvent, id: PaneId): void {
    const handle = e.currentTarget as HTMLElement;
    const startX = e.clientX;
    const startY = e.clientY;
    handle.setPointerCapture(e.pointerId);

    function move(ev: PointerEvent): void {
      if (!moving) {
        if (Math.hypot(ev.clientX - startX, ev.clientY - startY) < MOVE_THRESHOLD_PX) return;
        moving = { id, target: null, side: null };
      }
      const box = area.getBoundingClientRect();
      if (box.width === 0 || box.height === 0) return;
      const x = (ev.clientX - box.left) / box.width;
      const y = (ev.clientY - box.top) / box.height;
      const hit = paneAt(x, y, arranged.panes);
      moving =
        hit && hit.id !== id
          ? { id, target: hit.id, side: dropSide(x, y, hit.rect) }
          : { id, target: null, side: null };
    }
    function drop(): void {
      const m = moving;
      end();
      if (m && m.target !== null && m.side !== null) {
        layout = movePane(layout, m.id, m.target, m.side);
        focused = m.id;
      }
    }
    function key(ev: KeyboardEvent): void {
      // Before the press has become a drag, Escape is none of this listener's business.
      if (!moving || ev.key !== 'Escape') return;
      ev.preventDefault();
      ev.stopPropagation();
      end();
    }
    function end(): void {
      moving = null;
      cancelMove = null;
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', drop);
      handle.removeEventListener('pointercancel', end);
      handle.removeEventListener('lostpointercapture', end);
      window.removeEventListener('keydown', key, true);
    }
    cancelMove = end;
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', drop);
    handle.addEventListener('pointercancel', end);
    handle.addEventListener('lostpointercapture', end);
    window.addEventListener('keydown', key, true);
  }

  // The dragged pane can vanish mid-drag (its session exits, or its tab closes): once
  // it's no longer in the layout, end the drag rather than leave the preview and the
  // capture-phase Escape listener dangling.
  $effect(() => {
    if (moving && !arranged.panes.some((p) => p.id === moving!.id)) cancelMove?.();
  });

  onDestroy(() => cancelMove?.());

  const pct = (n: number): string => `${n * 100}%`;
</script>

<!-- bg-surface fills behind the macOS traffic lights (no seam). Text selection stays
     disabled app-wide (app.css); the terminal is the one selectable surface, handled
     by xterm's own selection (not CSS). -->
<div class="absolute inset-0 overflow-hidden bg-surface {active ? '' : 'hidden'}">
  <!-- Inset via this wrapper, not the xterm host: padding on the element xterm mounts
       into makes FitAddon over-size, sliding the last row under the status bar. The top
       inset clears the macOS traffic-light strip; the bottom gap clears the footer. -->
  <div class="h-full w-full" style="padding: max(var(--titlebar-h), 0.75rem) 0.5rem 1rem;">
    <div bind:this={area} class="relative h-full w-full">
      {#each arranged.panes as p (p.id)}
        <div
          class="absolute {moving?.id === p.id ? 'opacity-50' : ''}"
          data-pane={p.id}
          style="left: {pct(p.rect.x)}; top: {pct(p.rect.y)}; width: {pct(p.rect.w)}; height: {pct(p.rect.h)};"
        >
          <TerminalPane
            hostName={session.hostName}
            {active}
            focused={focused === p.id}
            framed={arranged.panes.length > 1}
            titled={arranged.panes.length > 1}
            status={statuses[p.id] ?? 'connecting'}
            onStatus={(s) => (statuses[p.id] = s)}
            onFocus={() => (focused = p.id)}
            onSplit={(dir) => splitPane(p.id, dir)}
            onClose={() => closePane(p.id)}
            onMoveStart={(e) => startMove(e, p.id)}
          />
        </div>
      {/each}
      {#each arranged.dividers as d (d.path.join('/'))}
        <div
          role="separator"
          aria-orientation={d.dir === 'row' ? 'vertical' : 'horizontal'}
          aria-label="Resize panes"
          class="absolute z-20 flex {d.dir === 'row'
            ? 'w-2 -translate-x-1/2 cursor-col-resize justify-center'
            : 'h-2 -translate-y-1/2 cursor-row-resize items-center'}"
          style="left: {pct(d.rect.x)}; top: {pct(d.rect.y)}; {d.dir === 'row'
            ? `height: ${pct(d.rect.h)}`
            : `width: ${pct(d.rect.w)}`};"
          onpointerdown={(e) => startDrag(e, d)}
        >
          <div class={d.dir === 'row' ? 'h-full border-l border-default' : 'w-full border-t border-default'}></div>
        </div>
      {/each}
      {#if preview}
        <!-- A full-opacity border reads clearly in both themes; the fill alone stays
             faint (a dimmed border, as opacity on the whole box would give, nearly
             disappears in dark mode). -->
        <div
          class="pointer-events-none absolute z-30 overflow-hidden rounded border-2 border-accent"
          style="left: {pct(preview.x)}; top: {pct(preview.y)}; width: {pct(preview.w)}; height: {pct(preview.h)};"
        >
          <div class="absolute inset-0 bg-accent opacity-20"></div>
        </div>
      {/if}
    </div>
  </div>
</div>
