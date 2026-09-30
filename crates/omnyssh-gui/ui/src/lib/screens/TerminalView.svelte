<script lang="ts">
  // A terminal tab (tech-gui.md §3.1): one or more panes, each its own SSH session to
  // the tab's host, laid out by a split tree the user can nest and resize (spec C).
  // Panes are positioned absolutely from the tree and keyed by id, so a split or a close
  // never remounts a pane — its xterm and session live on. Kept mounted for the tab's
  // whole life, hidden when another entity is active.
  import { untrack } from 'svelte';
  import { closeSession } from '$lib/stores/navigation';
  import { sessions, combineStatus, type Session, type SessionStatus } from '$lib/stores/sessions';
  import TerminalPane from './TerminalPane.svelte';
  import {
    arrange,
    clampRatio,
    neighbor,
    remove,
    setRatio,
    split,
    type DividerRect,
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
    }
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('pointercancel', end);
  }

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
          class="absolute"
          style="left: {pct(p.rect.x)}; top: {pct(p.rect.y)}; width: {pct(p.rect.w)}; height: {pct(p.rect.h)};"
        >
          <TerminalPane
            hostName={session.hostName}
            {active}
            focused={focused === p.id}
            framed={arranged.panes.length > 1}
            onStatus={(s) => (statuses[p.id] = s)}
            onFocus={() => (focused = p.id)}
            onSplit={(dir) => splitPane(p.id, dir)}
            onClose={() => closePane(p.id)}
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
    </div>
  </div>
</div>
