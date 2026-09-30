<script lang="ts">
  // Settings → Keyboard shortcuts: the terminal chords, each re-recorded by pressing it,
  // cleared (the keys then reach the shell), or reset. While recording, keydown is caught
  // on the window's capture phase and stopped, so the app's own chords (Ctrl+B, Ctrl+K)
  // don't fire.
  import { isMac } from '$lib/platform';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import {
    chordFromEvent,
    defaultBindings,
    formatChord,
    TERMINAL_ACTIONS,
    validateChord,
    type TerminalAction
  } from './terminalShortcuts';

  const defaults = defaultBindings(isMac);
  let recording = $state<TerminalAction | null>(null);
  let note = $state<{ action: TerminalAction; text: string; error: boolean } | null>(null);

  function record(action: TerminalAction): void {
    note = null;
    recording = recording === action ? null : action;
  }

  function onKeydown(e: KeyboardEvent): void {
    const action = recording;
    if (!action) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      recording = null;
      return;
    }
    const chord = chordFromEvent(e);
    if (!chord) return; // a lone modifier: keep listening
    const result = validateChord(action, chord, $terminalShortcuts);
    if (!result.ok) {
      note = { action, text: result.error, error: true };
      return; // still recording
    }
    terminalShortcuts.set(action, chord);
    note = result.warning ? { action, text: result.warning, error: false } : null;
    recording = null;
  }

  const btn =
    'rounded-full border border-default px-3 py-1 text-xs text-muted transition hover:border-strong ' +
    'hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus';
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="space-y-3">
  {#each TERMINAL_ACTIONS as { action, label } (action)}
    {@const chord = $terminalShortcuts[action]}
    <div>
      <div class="flex items-center justify-between gap-4">
        <p class="text-sm">{label}</p>
        <div class="flex items-center gap-2">
          <kbd
            class="min-w-[9rem] rounded-md bg-surface-inset px-2 py-1 text-center font-mono text-xs {recording ===
            action
              ? 'ring-2 ring-focus'
              : ''}"
          >
            {recording === action ? 'Press keys… (Esc cancels)' : chord ? formatChord(chord, isMac) : 'None'}
          </kbd>
          <button type="button" class={btn} onclick={() => record(action)}>
            {recording === action ? 'Cancel' : 'Change'}
          </button>
          <button
            type="button"
            class={btn}
            disabled={chord === null}
            onclick={() => terminalShortcuts.set(action, null)}
          >
            Clear
          </button>
          <button
            type="button"
            class={btn}
            disabled={chord === defaults[action]}
            onclick={() => terminalShortcuts.reset(action)}
          >
            Reset
          </button>
        </div>
      </div>
      {#if note?.action === action}
        <p class="mt-1 text-right text-xs {note.error ? 'text-status-crit' : 'text-status-warn'}">
          {note.text}
        </p>
      {/if}
    </div>
  {/each}
  <div class="flex items-center justify-between gap-4 pt-1">
    <p class="text-xs text-muted">A cleared shortcut sends its keys to the shell instead.</p>
    <button type="button" class={btn} onclick={() => terminalShortcuts.resetAll()}>Reset all</button>
  </div>
</div>
