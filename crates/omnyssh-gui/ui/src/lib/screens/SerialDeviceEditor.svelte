<script lang="ts">
  // Add / edit / duplicate a saved serial device. Validation lives in `deviceFromFields`;
  // the parent persists and reloads, and a rejected save shows inline without closing.
  import type { SerialDeviceDto } from '$lib/bindings';
  import Modal from '$lib/components/Modal.svelte';
  import { Button } from '$lib/theme';
  import SerialLineFields from './SerialLineFields.svelte';
  import { deviceFromFields, type SerialDeviceFields } from './serialForm';

  let {
    mode,
    initial,
    taken,
    onSubmit,
    onCancel
  }: {
    mode: 'add' | 'edit';
    initial: SerialDeviceFields;
    /** The other devices' names. */
    taken: string[];
    onSubmit: (device: SerialDeviceDto) => Promise<void>;
    onCancel: () => void;
  } = $props();

  // Seeded once; the editor is remounted per open.
  // svelte-ignore state_referenced_locally
  let fields = $state<SerialDeviceFields>({ ...initial });
  let error = $state<string | null>(null);
  let saving = $state(false);

  async function save(): Promise<void> {
    const result = deviceFromFields(fields, taken);
    if (!result.ok) {
      error = result.error;
      return;
    }
    error = null;
    saving = true;
    try {
      await onSubmit(result.device);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      saving = false;
    }
  }

  const title = $derived(mode === 'add' ? 'Add serial device' : 'Edit serial device');
  const label = 'block space-y-1 text-xs font-medium text-muted';
  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint disabled:opacity-60';
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
    <div class="min-h-0 flex-1 space-y-3.5 overflow-y-auto px-5 py-4">
      <label class={label}>
        <span>Name {mode === 'edit' ? '(fixed)' : ''}</span>
        <input bind:value={fields.name} class={field} disabled={mode === 'edit'} placeholder="ESP32 devkit" />
      </label>
      <SerialLineFields bind:fields />
      <label class={label}>
        <span>Notes</span>
        <input bind:value={fields.notes} class={field} placeholder="Optional" />
      </label>
      {#if error}
        <p class="text-xs text-status-crit">{error}</p>
      {/if}
    </div>
    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
      <Button onclick={onCancel}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={saving}>Save</Button>
    </footer>
  </form>
</Modal>
