<script lang="ts">
  // A one-field dialog (name a group, rename it). The parent validates in `onSubmit`;
  // a returned message shows inline and keeps the dialog open.
  import { onMount } from 'svelte';
  import Modal from './Modal.svelte';
  import { Button } from '$lib/theme';

  let {
    title,
    label,
    initial = '',
    submitLabel = 'Save',
    onSubmit,
    onCancel
  }: {
    title: string;
    label: string;
    initial?: string;
    submitLabel?: string;
    onSubmit: (value: string) => Promise<string | null>;
    onCancel: () => void;
  } = $props();

  // svelte-ignore state_referenced_locally
  let value = $state(initial);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let input = $state<HTMLInputElement>();
  onMount(() => input?.select());

  async function submit(): Promise<void> {
    busy = true;
    try {
      error = await onSubmit(value);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }

  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus';
</script>

<Modal label={title} onClose={onCancel}>
  <form
    class="space-y-3 px-5 py-4"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
  >
    <h2 class="text-sm font-semibold">{title}</h2>
    <label class="block space-y-1 text-xs font-medium text-muted">
      <span>{label}</span>
      <input bind:this={input} bind:value class={field} />
    </label>
    {#if error}<p class="text-xs text-status-crit">{error}</p>{/if}
    <div class="flex justify-end gap-2 pt-1">
      <Button onclick={onCancel}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={busy}>{submitLabel}</Button>
    </div>
  </form>
</Modal>
