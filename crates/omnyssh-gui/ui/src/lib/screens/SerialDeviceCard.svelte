<script lang="ts">
  // A saved serial device on the dashboard: its name, line settings and notes, with
  // monitor / term to open it and edit / duplicate / delete. No live state — the card
  // only shows what is saved.
  import type { SerialDeviceDto } from '$lib/bindings';
  import { Surface, Icon } from '$lib/theme';
  import type { SerialMode } from '$lib/stores/sessions';
  import { describeLine } from './serialForm';

  let {
    device,
    onOpen,
    onEdit,
    onDuplicate,
    onDelete
  }: {
    device: SerialDeviceDto;
    onOpen: (mode: SerialMode) => void;
    onEdit: () => void;
    onDuplicate: () => void;
    onDelete: () => void;
  } = $props();

  const OPEN: { mode: SerialMode; label: string }[] = [
    { mode: 'monitor', label: 'monitor' },
    { mode: 'terminal', label: 'term' }
  ];

  const pill =
    'inline-flex items-center gap-1.5 rounded-full border border-default px-2.5 py-1 text-xs ' +
    'font-medium text-muted transition hover:border-strong hover:bg-accent hover:text-accent-fg ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  const iconBtn =
    'grid h-7 w-7 place-items-center rounded-lg text-muted transition hover:bg-surface-inset ' +
    'hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

<Surface class="flex flex-col gap-3 p-5">
  <div class="flex min-w-0 items-start gap-2.5">
    <span class="mt-0.5 shrink-0 text-muted"><Icon name="serial" size={16} /></span>
    <div class="min-w-0 flex-1">
      <div class="flex min-w-0 items-center gap-2">
        <span class="truncate font-medium" title={device.name}>{device.name}</span>
        <span class="shrink-0 rounded-full border border-default px-1.5 py-0.5 text-[10px] text-faint">
          serial
        </span>
      </div>
      <div class="truncate font-mono text-xs text-faint">{describeLine(device.config)}</div>
    </div>
  </div>
  <div class="flex flex-wrap items-center gap-1.5">
    {#each OPEN as o (o.mode)}
      <button type="button" class={pill} title="{o.label} on {device.name}" onclick={() => onOpen(o.mode)}>
        <Icon name={o.mode === 'monitor' ? 'serial' : 'terminal'} size={13} />
        {o.label}
      </button>
    {/each}
    <span class="flex-1"></span>
    <button type="button" class={iconBtn} title="Edit {device.name}" aria-label="Edit {device.name}" onclick={onEdit}>
      <Icon name="edit" size={14} />
    </button>
    <button
      type="button"
      class={iconBtn}
      title="Duplicate {device.name}"
      aria-label="Duplicate {device.name}"
      onclick={onDuplicate}
    >
      <Icon name="copy" size={14} />
    </button>
    <button type="button" class={iconBtn} title="Delete {device.name}" aria-label="Delete {device.name}" onclick={onDelete}>
      <Icon name="trash" size={14} />
    </button>
  </div>
  {#if device.notes}
    <p class="line-clamp-2 text-xs text-muted" title={device.notes}>{device.notes}</p>
  {/if}
</Surface>
