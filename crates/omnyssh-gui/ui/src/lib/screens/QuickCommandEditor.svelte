<script lang="ts">
  // Add or edit one quick command: label, text or hex payload, and what follows it.
  import { onMount } from 'svelte';
  import type { QuickCommandDto } from '$lib/bindings';
  import Modal from '$lib/components/Modal.svelte';
  import Select from '$lib/components/Select.svelte';
  import { Button } from '$lib/theme';
  import { commandFromFields, parseHex, type CommandFields } from './quickCommand';

  let {
    mode,
    initial,
    onSubmit,
    onCancel
  }: {
    mode: 'add' | 'edit';
    initial: CommandFields;
    onSubmit: (cmd: QuickCommandDto) => Promise<void>;
    onCancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let fields = $state<CommandFields>({ ...initial });
  let error = $state<string | null>(null);
  let saving = $state(false);
  let labelEl = $state<HTMLInputElement>();
  onMount(() => labelEl?.focus());

  const hexBad = $derived(fields.kind === 'hex' && fields.payload.trim() !== '' && !parseHex(fields.payload));

  async function save(): Promise<void> {
    const result = commandFromFields(fields);
    if (!result.ok) {
      error = result.error;
      return;
    }
    error = null;
    saving = true;
    try {
      await onSubmit(result.command);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  const title = $derived(mode === 'add' ? 'Add quick command' : 'Edit quick command');
  const label = 'block space-y-1 text-xs font-medium text-muted';
  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint';
</script>

<Modal label={title} onClose={onCancel}>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
    class="flex min-h-0 flex-col"
  >
    <header class="border-b border-default px-5 py-3.5">
      <h2 class="text-sm font-semibold">{title}</h2>
    </header>
    <div class="space-y-3.5 px-5 py-4">
      <label class={label}>
        <span>Label</span>
        <input bind:this={labelEl} bind:value={fields.label} class={field} placeholder="disk usage" />
      </label>
      <div class="grid grid-cols-2 gap-3">
        <label class={label}>
          <span>Send as</span>
          <Select bind:value={fields.kind} class={field}>
            <option value="text">Text</option>
            <option value="hex">Hex bytes</option>
          </Select>
        </label>
        <label class={label}>
          <span>Then send</span>
          <Select bind:value={fields.ending} class={field}>
            <option value="cr">CR (Enter)</option>
            <option value="lf">LF</option>
            <option value="crlf">CR LF</option>
            <option value="none">Nothing</option>
          </Select>
        </label>
      </div>
      <label class={label}>
        <span>{fields.kind === 'hex' ? 'Bytes' : 'Text'}</span>
        <textarea
          bind:value={fields.payload}
          rows="3"
          class="{field} font-mono {hexBad ? 'ring-2 ring-status-crit' : ''}"
          placeholder={fields.kind === 'hex' ? '55 AA 01 00' : 'df -h'}
        ></textarea>
      </label>
      {#if hexBad}
        <p class="text-xs text-status-crit">Hex must be whole bytes, like 55 AA 01</p>
      {/if}
      {#if error}<p class="text-xs text-status-crit">{error}</p>{/if}
    </div>
    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
      <Button onclick={onCancel}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={saving}>Save</Button>
    </footer>
  </form>
</Modal>
