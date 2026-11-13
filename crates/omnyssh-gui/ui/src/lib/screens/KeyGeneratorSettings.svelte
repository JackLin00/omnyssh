<script lang="ts">
  // Settings › Key generators: the switch (off by default; off, nothing changes
  // anywhere) and, on, the generators with add / edit / delete. Renaming or deleting
  // one follows through to the hosts that name it, so the hosts are reloaded after.
  import { onMount } from 'svelte';
  import type { KeyGeneratorDto } from '$lib/bindings';
  import { Button, Icon } from '$lib/theme';
  import Modal from '$lib/components/Modal.svelte';
  import { deleteKeyGenerator, reloadHosts, saveKeyGenerator, setKeyGeneratorsEnabled } from '$lib/ipc/commands';
  import { hosts } from '$lib/stores/hosts';
  import { keyGenerators, loadKeyGenerators } from '$lib/stores/keyGenerators';
  import { lastError } from '$lib/stores/notifications';
  import KeyGeneratorEditor from './KeyGeneratorEditor.svelte';
  import { emptyGeneratorFields, hostsUsing } from './keyGenForm';

  onMount(() => void loadKeyGenerators());

  type Dialog = { kind: 'add' } | { kind: 'edit'; generator: KeyGeneratorDto } | { kind: 'delete'; name: string };
  let dialog = $state<Dialog | null>(null);

  const enabled = $derived($keyGenerators?.enabled ?? false);
  const generators = $derived($keyGenerators?.generators ?? []);
  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function toggle(): Promise<void> {
    try {
      await setKeyGeneratorsEnabled(!enabled);
    } catch (e) {
      lastError.set(`Key generators: ${message(e)}`);
    }
    await loadKeyGenerators();
  }

  // A save error stays in the editor; once saved, the editor closes, and a failed
  // host reload is only reported — the save itself went through.
  async function save(previous: string | null, generator: KeyGeneratorDto): Promise<void> {
    await saveKeyGenerator(previous, generator);
    dialog = null;
    await loadKeyGenerators();
    if (previous !== null && previous !== generator.name) await refreshHosts();
  }

  async function remove(name: string): Promise<void> {
    dialog = null;
    try {
      await deleteKeyGenerator(name);
    } catch (e) {
      lastError.set(`Delete key generator: ${message(e)}`);
      return;
    }
    await loadKeyGenerators();
    await refreshHosts();
  }

  /** The hosts again, after a rename or delete changed them on disk. */
  async function refreshHosts(): Promise<void> {
    try {
      await reloadHosts();
    } catch (e) {
      lastError.set(`Reload hosts: ${message(e)}`);
    }
  }

  const iconBtn =
    'grid h-7 w-7 place-items-center rounded-lg text-muted transition hover:bg-surface-inset hover:text-fg ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

<div class="space-y-3">
  <div class="flex items-center justify-between gap-4">
    <div class="min-w-0">
      <p class="text-sm">Use key generators</p>
      <p class="text-xs text-muted">
        Hosts set up with one try the key it makes first, then log in as usual.
      </p>
    </div>
    <button
      type="button"
      role="switch"
      aria-checked={enabled}
      aria-label="Use key generators"
      onclick={toggle}
      class="relative h-6 w-11 shrink-0 rounded-full transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {enabled
        ? 'bg-accent'
        : 'bg-surface-inset'}"
    >
      <span
        class="absolute top-0.5 h-5 w-5 rounded-full bg-surface shadow-soft transition-[left] {enabled
          ? 'left-[1.375rem]'
          : 'left-0.5'}"
      ></span>
    </button>
  </div>

  {#if enabled}
    <ul class="divide-y divide-default rounded-lg border border-default" aria-label="Key generators">
      {#each generators as g (g.name)}
        <li class="flex items-center gap-2 px-3 py-2">
          <span class="min-w-0 flex-1 truncate text-sm">{g.name}</span>
          <span class="shrink-0 text-xs text-faint">
            {hostsUsing($hosts, g.name).length} host{hostsUsing($hosts, g.name).length === 1 ? '' : 's'}
          </span>
          <button type="button" class={iconBtn} title="Edit {g.name}" aria-label="Edit {g.name}" onclick={() => (dialog = { kind: 'edit', generator: g })}>
            <Icon name="edit" size={14} />
          </button>
          <button type="button" class={iconBtn} title="Delete {g.name}" aria-label="Delete {g.name}" onclick={() => (dialog = { kind: 'delete', name: g.name })}>
            <Icon name="trash" size={14} />
          </button>
        </li>
      {:else}
        <li class="px-3 py-2 text-xs text-muted">No key generators yet.</li>
      {/each}
    </ul>
    <Button variant="ghost" onclick={() => (dialog = { kind: 'add' })}>
      <Icon name="plus" size={14} /> Add key generator
    </Button>
  {/if}
</div>

{#if dialog?.kind === 'add'}
  <KeyGeneratorEditor
    mode="add"
    initial={emptyGeneratorFields()}
    taken={generators.map((g) => g.name)}
    onSubmit={(g) => save(null, g)}
    onCancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'edit'}
  {@const from = dialog.generator}
  <KeyGeneratorEditor
    mode="edit"
    initial={{ name: from.name, code: from.code, credential: from.credential }}
    taken={generators.filter((g) => g.name !== from.name).map((g) => g.name)}
    onSubmit={(g) => save(from.name, g)}
    onCancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'delete'}
  {@const name = dialog.name}
  {@const using = hostsUsing($hosts, name)}
  <Modal label="Delete key generator" onClose={() => (dialog = null)}>
    <div class="space-y-3 px-5 py-4">
      <h2 class="text-sm font-semibold">Delete key generator</h2>
      <p class="text-sm text-muted">Delete “{name}”?</p>
      {#if using.length > 0}
        <p class="text-sm text-muted">
          {using.join(', ')}
          {using.length === 1 ? 'uses' : 'use'} it, and will log in without it from now on.
        </p>
      {/if}
      <div class="flex justify-end gap-2 pt-1">
        <Button variant="ghost" onclick={() => (dialog = null)}>Cancel</Button>
        <Button variant="primary" onclick={() => remove(name)}>Delete</Button>
      </div>
    </div>
  </Modal>
{/if}
