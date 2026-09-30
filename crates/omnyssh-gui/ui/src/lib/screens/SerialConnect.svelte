<script lang="ts">
  // Pick a serial port and its line settings, then open a serial tab. Nothing is
  // saved: the defaults are the 115200 8N1 almost every MCU boots with.
  import Modal from '$lib/components/Modal.svelte';
  import Select from '$lib/components/Select.svelte';
  import { Button } from '$lib/theme';
  import { serialConnect } from '$lib/stores/serialConnect';
  import { spawnSession } from '$lib/stores/navigation';
  import type { SerialMode } from '$lib/stores/sessions';
  import type { EnterKey } from './serialFormat';
  import SerialLineFields from './SerialLineFields.svelte';
  import { emptyLineFields, lineConfig } from './serialForm';

  let fields = $state(emptyLineFields());
  let mode = $state('monitor');
  const config = $derived(lineConfig(fields));

  function connect(): void {
    if (!config) return;
    spawnSession('serial', `${config.port} · ${config.baudRate}`, {
      config,
      mode: mode as SerialMode,
      enter: fields.enter as EnterKey
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
      <SerialLineFields bind:fields showEnter={mode === 'terminal'} />
      <label class={label}>
        <span>Mode</span>
        <Select bind:value={mode} class={field}>
          <option value="monitor">Monitor (receive only)</option>
          <option value="terminal">Terminal</option>
        </Select>
      </label>
    </div>

    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
      <Button onclick={serialConnect.close}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={!config}>Open</Button>
    </footer>
  </form>
</Modal>
