<script lang="ts">
  // The command bar under a terminal or serial tab, the same for both: the current
  // group's commands as buttons that send their bytes to the tab's session — a click
  // sends the payload and its ending, Shift+click leaves a text command's ending off so
  // it can be edited first. Edit mode shows the edit / delete affordances and the group
  // actions; the group shown and the collapsed state are UI prefs shared by every tab.
  import type { QuickCommandDto } from '$lib/bindings';
  import { Icon } from '$lib/theme';
  import Select from '$lib/components/Select.svelte';
  import Modal from '$lib/components/Modal.svelte';
  import TextPrompt from '$lib/components/TextPrompt.svelte';
  import { Button } from '$lib/theme';
  import { isMac } from '$lib/platform';
  import { lastError } from '$lib/stores/notifications';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import {
    currentGroup,
    quickBarCollapsed,
    quickGroups,
    saveQuickGroups,
    selectedGroup
  } from '$lib/stores/quickCommands';
  import QuickCommandEditor from './QuickCommandEditor.svelte';
  import {
    addGroup,
    commandBytes,
    deleteCommand,
    deleteGroup,
    emptyCommandFields,
    fieldsFromCommand,
    renameGroup,
    upsertCommand
  } from './quickCommand';
  import { formatChord, QUICK_SLOTS, type QuickCommandAction } from './terminalShortcuts';

  let { enabled, onSend }: { enabled: boolean; onSend: (bytes: Uint8Array) => void } = $props();

  let editing = $state(false);
  type Dialog =
    | { kind: 'addGroup' }
    | { kind: 'renameGroup'; name: string }
    | { kind: 'deleteGroup'; name: string }
    | { kind: 'addCommand' }
    | { kind: 'editCommand'; index: number; command: QuickCommandDto };
  let dialog = $state<Dialog | null>(null);

  const group = $derived($currentGroup);

  function hint(index: number): string {
    if (index >= QUICK_SLOTS.length) return '';
    const chord = $terminalShortcuts[`quickCommand${index + 1}` as QuickCommandAction];
    return chord ? ` (${formatChord(chord, isMac)})` : '';
  }

  function run(cmd: QuickCommandDto, e: MouseEvent): void {
    if (editing || !enabled) return;
    const bytes = commandBytes(cmd, e.shiftKey);
    if (bytes && bytes.length > 0) onSend(bytes);
  }

  async function persist(next: typeof $quickGroups): Promise<void> {
    await saveQuickGroups(next);
  }

  async function removeCommand(index: number): Promise<void> {
    if (!group) return;
    try {
      await persist(deleteCommand($quickGroups, group.name, index));
    } catch (e) {
      lastError.set(e instanceof Error ? e.message : String(e));
    }
  }

  const btn =
    'inline-flex shrink-0 items-center gap-1 rounded-full border border-default px-2.5 py-0.5 text-xs ' +
    'text-muted transition hover:border-strong hover:bg-accent hover:text-accent-fg ' +
    'disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  const iconBtn =
    'grid h-6 w-6 shrink-0 place-items-center rounded text-muted transition hover:bg-surface-inset ' +
    'hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

{#if $quickBarCollapsed}
  <div class="flex shrink-0 justify-end border-t border-default px-2 py-0.5">
    <button type="button" class={iconBtn} title="Show quick commands" aria-label="Show quick commands" onclick={() => quickBarCollapsed.set(false)}>
      <Icon name="chevronUp" size={14} />
    </button>
  </div>
{:else}
  <div class="flex shrink-0 items-center gap-2 border-t border-default px-2 py-1.5" role="toolbar" aria-label="Quick commands">
    {#if $quickGroups.length === 0}
      <span class="text-xs text-muted">Add a group to start</span>
      <button type="button" class={btn} onclick={() => (dialog = { kind: 'addGroup' })}>
        <Icon name="plus" size={12} /> Group
      </button>
    {:else}
      <div class="w-32 shrink-0">
        <Select
          value={group?.name ?? ''}
          onchange={(e) => selectedGroup.set((e.currentTarget as HTMLSelectElement).value)}
          aria-label="Quick command group"
          class="w-full rounded-md bg-surface-inset py-1 pl-2 text-xs text-fg outline-none focus-visible:ring-2 focus-visible:ring-focus"
        >
          {#each $quickGroups as g (g.name)}<option value={g.name}>{g.name}</option>{/each}
        </Select>
      </div>
      {#if editing}
        <button type="button" class={iconBtn} title="New group" aria-label="New group" onclick={() => (dialog = { kind: 'addGroup' })}>
          <Icon name="plus" size={13} />
        </button>
        {#if group}
          <button type="button" class={iconBtn} title="Rename group" aria-label="Rename group" onclick={() => (dialog = { kind: 'renameGroup', name: group.name })}>
            <Icon name="edit" size={13} />
          </button>
          <button type="button" class={iconBtn} title="Delete group" aria-label="Delete group" onclick={() => (dialog = { kind: 'deleteGroup', name: group.name })}>
            <Icon name="trash" size={13} />
          </button>
        {/if}
      {/if}
      <div class="flex min-w-0 flex-1 items-center gap-1.5 overflow-x-auto">
        {#each group?.commands ?? [] as cmd, i (i)}
          <span class="inline-flex shrink-0 items-center">
            <button
              type="button"
              class={btn}
              disabled={!enabled && !editing}
              title="{cmd.kind === 'hex' ? cmd.payload : cmd.payload || '(ending only)'}{hint(i)}"
              onclick={(e) => (editing ? (dialog = { kind: 'editCommand', index: i, command: cmd }) : run(cmd, e))}
            >
              {cmd.label}
            </button>
            {#if editing}
              <button type="button" class={iconBtn} title="Delete {cmd.label}" aria-label="Delete {cmd.label}" onclick={() => removeCommand(i)}>
                <Icon name="close" size={12} />
              </button>
            {/if}
          </span>
        {/each}
        {#if group}
          <button type="button" class={iconBtn} title="Add quick command" aria-label="Add quick command" onclick={() => (dialog = { kind: 'addCommand' })}>
            <Icon name="plus" size={13} />
          </button>
        {/if}
      </div>
    {/if}
    <button
      type="button"
      class="{iconBtn} {editing ? 'bg-accent text-accent-fg hover:bg-accent hover:text-accent-fg' : ''}"
      title={editing ? 'Done editing' : 'Edit quick commands'}
      aria-label={editing ? 'Done editing' : 'Edit quick commands'}
      aria-pressed={editing}
      onclick={() => (editing = !editing)}
    >
      <Icon name={editing ? 'check' : 'edit'} size={13} />
    </button>
    <button type="button" class={iconBtn} title="Hide quick commands" aria-label="Hide quick commands" onclick={() => quickBarCollapsed.set(true)}>
      <Icon name="chevronDown" size={14} />
    </button>
  </div>
{/if}

{#if dialog?.kind === 'addGroup'}
  <TextPrompt
    title="New group"
    label="Name"
    submitLabel="Add"
    onSubmit={async (value) => {
      const r = addGroup($quickGroups, value);
      if (!r.ok) return r.error;
      await persist(r.groups);
      selectedGroup.set(value.trim());
      dialog = null;
      return null;
    }}
    onCancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'renameGroup'}
  {@const from = dialog.name}
  <TextPrompt
    title="Rename group"
    label="Name"
    initial={from}
    onSubmit={async (value) => {
      const r = renameGroup($quickGroups, from, value);
      if (!r.ok) return r.error;
      await persist(r.groups);
      selectedGroup.set(value.trim());
      dialog = null;
      return null;
    }}
    onCancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'deleteGroup'}
  {@const name = dialog.name}
  <Modal label="Delete group" onClose={() => (dialog = null)}>
    <div class="space-y-3 px-5 py-4">
      <h2 class="text-sm font-semibold">Delete group</h2>
      <p class="text-sm text-muted">Delete the group <span class="font-medium text-fg">{name}</span> and its commands?</p>
      <div class="flex justify-end gap-2 pt-1">
        <Button variant="ghost" onclick={() => (dialog = null)}>Cancel</Button>
        <Button
          variant="primary"
          onclick={async () => {
            try {
              await persist(deleteGroup($quickGroups, name));
            } catch (e) {
              lastError.set(e instanceof Error ? e.message : String(e));
            }
            dialog = null;
          }}
        >
          Delete
        </Button>
      </div>
    </div>
  </Modal>
{:else if dialog?.kind === 'addCommand' && group}
  {@const name = group.name}
  <QuickCommandEditor
    mode="add"
    initial={emptyCommandFields()}
    onSubmit={async (cmd) => {
      await persist(upsertCommand($quickGroups, name, null, cmd));
      dialog = null;
    }}
    onCancel={() => (dialog = null)}
  />
{:else if dialog?.kind === 'editCommand' && group}
  {@const name = group.name}
  {@const index = dialog.index}
  <QuickCommandEditor
    mode="edit"
    initial={fieldsFromCommand(dialog.command)}
    onSubmit={async (cmd) => {
      await persist(upsertCommand($quickGroups, name, index, cmd));
      dialog = null;
    }}
    onCancel={() => (dialog = null)}
  />
{/if}
