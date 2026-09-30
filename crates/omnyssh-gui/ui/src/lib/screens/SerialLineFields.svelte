<script lang="ts">
  // The serial line settings, shared by the ad-hoc Serial dialog and the saved device
  // editor. Scans the ports on mount; a port that isn't plugged in right now (a saved
  // device's) stays selectable, marked as such.
  import { onMount } from 'svelte';
  import Select from '$lib/components/Select.svelte';
  import { Button, Icon } from '$lib/theme';
  import { serialListPorts } from '$lib/ipc/commands';
  import type { SerialPortDto } from '$lib/bindings';
  import type { SerialLineFields } from './serialForm';

  let {
    fields = $bindable(),
    showEnter = true
  }: { fields: SerialLineFields; showEnter?: boolean } = $props();

  const BAUDS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000];

  let ports = $state<SerialPortDto[]>([]);
  let error = $state<string | null>(null);
  const missing = $derived(fields.port !== '' && !ports.some((p) => p.name === fields.port));

  async function rescan(): Promise<void> {
    try {
      ports = await serialListPorts();
      error = null;
      // Only an empty choice is filled in: a saved device keeps its port even unplugged.
      if (fields.port === '') fields.port = ports[0]?.name ?? '';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  onMount(rescan);

  const label = 'block space-y-1 text-xs font-medium text-muted';
  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint';
</script>

<div class="flex items-end gap-2">
  <label class="{label} flex-1">
    <span>Port</span>
    <Select bind:value={fields.port} class={field}>
      {#if missing}<option value={fields.port}>{fields.port} — not connected</option>{/if}
      {#each ports as p (p.name)}
        <option value={p.name}>{p.name}{p.description ? ` — ${p.description}` : ''}</option>
      {/each}
    </Select>
  </label>
  <Button variant="icon" title="Rescan ports" onclick={rescan}>
    <Icon name="refresh" />
  </Button>
</div>
{#if error}
  <p class="text-xs text-status-crit">{error}</p>
{:else if ports.length === 0}
  <p class="text-xs text-muted">No serial ports found. Plug the adapter in and rescan.</p>
{/if}

<div class="grid grid-cols-2 gap-3">
  <label class={label}>
    <span>Baud rate</span>
    <input bind:value={fields.baud} list="serial-bauds" inputmode="numeric" class="{field} font-mono" />
    <datalist id="serial-bauds">
      {#each BAUDS as b (b)}<option value={String(b)}></option>{/each}
    </datalist>
  </label>
  <label class={label}>
    <span>Data bits</span>
    <Select bind:value={fields.dataBits} class={field}>
      {#each ['8', '7', '6', '5'] as d (d)}<option value={d}>{d}</option>{/each}
    </Select>
  </label>
  <label class={label}>
    <span>Parity</span>
    <Select bind:value={fields.parity} class={field}>
      <option value="none">None</option>
      <option value="odd">Odd</option>
      <option value="even">Even</option>
    </Select>
  </label>
  <label class={label}>
    <span>Stop bits</span>
    <Select bind:value={fields.stopBits} class={field}>
      <option value="one">1</option>
      <option value="two">2</option>
    </Select>
  </label>
  <label class={label}>
    <span>Flow control</span>
    <Select bind:value={fields.flow} class={field}>
      <option value="none">None</option>
      <option value="software">XON/XOFF</option>
      <option value="hardware">RTS/CTS</option>
    </Select>
  </label>
  {#if showEnter}
    <label class={label}>
      <span>Enter sends</span>
      <Select bind:value={fields.enter} class={field}>
        <option value="cr">CR (\r)</option>
        <option value="lf">LF (\n)</option>
        <option value="crlf">CR LF (\r\n)</option>
      </Select>
    </label>
  {/if}
</div>
