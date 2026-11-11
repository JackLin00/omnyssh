<script lang="ts">
  // A view's drag-and-drop upload UI (plan Q): the overlay while files are dragged over
  // it, and the progress bar with Cancel while its upload runs. Placed inside the view's
  // root, which must be `position: relative`.
  import { dropHover } from './dropTargets';
  import { dropError, dropNote, progressLine, protocolName, uploads } from '$lib/stores/uploads';

  let {
    root,
    sessionId,
    onCancel
  }: {
    /** The view's registered drop target. */
    root: Element | undefined;
    /** The backend session id; undefined while connecting. */
    sessionId: number | undefined;
    onCancel: () => void;
  } = $props();

  const hover = $derived($dropHover && root && $dropHover.el === root ? $dropHover : null);
  const refusal = $derived(hover ? dropError(hover.protocol, hover.count) : null);
  const note = $derived(hover ? dropNote(hover.protocol, hover.count) : null);
  const view = $derived(sessionId == null ? undefined : $uploads.get(sessionId));
  const line = $derived(view ? progressLine(view) : null);
</script>

{#if hover}
  <div
    class="pointer-events-none absolute inset-0 z-40 grid place-items-center rounded"
    style="background: color-mix(in srgb, var(--bg) 70%, transparent);"
    data-testid="drop-overlay"
  >
    <div class="max-w-[80%] rounded-lg bg-surface-raised px-4 py-3 text-center text-sm text-fg shadow-soft">
      {#if refusal}
        <p class="text-status-crit">{refusal}</p>
      {:else}
        <p>Drop to upload with {protocolName(hover.protocol)} to {hover.label}</p>
        {#if note}<p class="mt-1 text-xs text-muted">{note}</p>{/if}
      {/if}
    </div>
  </div>
{/if}

{#if view && line}
  <div
    class="absolute inset-x-1.5 bottom-1.5 z-30 flex items-center gap-2 rounded-md bg-surface-raised px-2.5 py-1.5 text-xs text-fg shadow-soft"
    role="status"
    aria-label="Upload"
  >
    <span class="shrink-0 font-medium">{protocolName(view.protocol)}</span>
    <div class="min-w-0 flex-1">
      <div class="truncate" data-testid="upload-text">{line.text}</div>
      <div class="mt-1 h-1 overflow-hidden rounded-full bg-surface-inset">
        <div
          class="h-full rounded-full bg-accent transition-[width] {line.fraction === null ? 'w-1/3 animate-pulse' : ''}"
          style={line.fraction === null ? '' : `width: ${line.fraction * 100}%`}
        ></div>
      </div>
    </div>
    <button
      type="button"
      class="shrink-0 rounded px-2 py-0.5 text-xs text-muted transition hover:bg-surface hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
      onclick={onCancel}
    >
      Cancel
    </button>
  </div>
{/if}
