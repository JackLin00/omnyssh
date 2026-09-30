<script lang="ts">
  // Pick a serial port and its line settings, then open a serial tab. Nothing is
  // saved: the defaults are the 115200 8N1 almost every MCU boots with.
  import { onMount } from 'svelte';
  import Modal from '$lib/components/Modal.svelte';
  import Select from '$lib/components/Select.svelte';
  import { Button, Icon } from '$lib/theme';
  import { serialConnect } from '$lib/stores/serialConnect';
  import { spawnSession } from '$lib/stores/navigation';
  import { serialListPorts } from '$lib/ipc/commands';
  import type { FlowControlDto, ParityDto, SerialPortDto, StopBitsDto } from '$lib/bindings';
  import type { SerialMode } from '$lib/stores/sessions';
  import type { EnterKey } from './serialFormat';

  const BAUDS = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600, 1500000];

  let ports = $state<SerialPortDto[]>([]);
  let error = $state<string | null>(null);
  // `Select` binds strings; each value is narrowed to its DTO type in `connect`.
  let port = $state('');
  let baud = $state('115200');
  let dataBits = $state('8');
  let parity = $state('none');
  let stopBits = $state('one');
  let flow = $state('none');
  let mode = $state('monitor');
  let enter = $state('cr');

  async function rescan(): Promise<void> {
    try {
      ports = await serialListPorts();
      error = null;
      if (!ports.some((p) => p.name === port)) port = ports[0]?.name ?? '';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }
  onMount(rescan);

  const baudRate = $derived(Number.parseInt(baud, 10));
  const valid = $derived(port !== '' && Number.isInteger(baudRate) && baudRate > 0);

  function connect(): void {
    if (!valid) return;
    spawnSession('serial', `${port} · ${baudRate}`, {
      config: {
        port,
        baudRate,
        dataBits: Number(dataBits),
        parity: parity as ParityDto,
        stopBits: stopBits as StopBitsDto,
        flowControl: flow as FlowControlDto
      },
      mode: mode as SerialMode,
      enter: enter as EnterKey
    });
    serialConnect.close();
  }

  const label = 'block space-y-1 text-xs font-medium text-muted';
  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint';
</script>

<Modal label="Open serial port" onClose={serialConnect.close}>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      connect();
    }}
    class="flex min-h-0 flex-col"
  >
    <header class="border-b border-default px-5 py-3.5">
      <h2 class="text-sm font-semibold">Open serial port</h2>
    </header>

    <div class="min-h-0 flex-1 space-y-3.5 overflow-y-auto px-5 py-4">
      <div class="flex items-end gap-2">
        <label class="{label} flex-1">
          <span>Port</span>
          <Select bind:value={port} class={field}>
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
          <input bind:value={baud} list="serial-bauds" inputmode="numeric" class="{field} font-mono" />
          <datalist id="serial-bauds">
            {#each BAUDS as b (b)}<option value={String(b)}></option>{/each}
          </datalist>
        </label>
        <label class={label}>
          <span>Data bits</span>
          <Select bind:value={dataBits} class={field}>
            {#each ['8', '7', '6', '5'] as d (d)}<option value={d}>{d}</option>{/each}
          </Select>
        </label>
        <label class={label}>
          <span>Parity</span>
          <Select bind:value={parity} class={field}>
            <option value="none">None</option>
            <option value="odd">Odd</option>
            <option value="even">Even</option>
          </Select>
        </label>
        <label class={label}>
          <span>Stop bits</span>
          <Select bind:value={stopBits} class={field}>
            <option value="one">1</option>
            <option value="two">2</option>
          </Select>
        </label>
        <label class={label}>
          <span>Flow control</span>
          <Select bind:value={flow} class={field}>
            <option value="none">None</option>
            <option value="software">XON/XOFF</option>
            <option value="hardware">RTS/CTS</option>
          </Select>
        </label>
        <label class={label}>
          <span>Mode</span>
          <Select bind:value={mode} class={field}>
            <option value="monitor">Monitor (receive only)</option>
            <option value="terminal">Terminal</option>
          </Select>
        </label>
        {#if mode === 'terminal'}
          <label class={label}>
            <span>Enter sends</span>
            <Select bind:value={enter} class={field}>
              <option value="cr">CR (\r)</option>
              <option value="lf">LF (\n)</option>
              <option value="crlf">CR LF (\r\n)</option>
            </Select>
          </label>
        {/if}
      </div>
    </div>

    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
      <Button onclick={serialConnect.close}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={!valid}>Open</Button>
    </footer>
  </form>
</Modal>
