<script lang="ts">
  // Settings → Window → Show / hide OmnySSH (plan J): the one system-wide hotkey that
  // brings the window forward or hides it. Recorded the same way as the terminal
  // chords (ShortcutSettings.svelte): keydown caught on the window's capture phase,
  // Esc or a click elsewhere cancels, a lone modifier keeps listening. While recording
  // the hotkey is unregistered — `setGlobalHotkey(null)` directly, bypassing the
  // preference — so the combo being pressed can never hide the window out from under
  // the recorder; `applyGlobalHotkey()` on the way out re-registers whatever the
  // preference ends up holding, recorded or unchanged.
  import { get } from 'svelte/store';
  import { isMac } from '$lib/platform';
  import { chordFromEvent, formatChord, parseChord } from './terminalShortcuts';
  import {
    applyGlobalHotkey,
    chordToAccelerator,
    DEFAULT_HOTKEY,
    globalHotkey,
    globalHotkeyError
  } from '$lib/stores/globalHotkey';
  import { setGlobalHotkey } from '$lib/ipc/commands';

  let recording = $state(false);
  let rowEl: HTMLElement | undefined;

  function enterRecording(): void {
    recording = true;
    void setGlobalHotkey(null);
  }

  /** Recording ends with nothing recorded (Esc, a click elsewhere, losing focus to
   *  another app): the preference never changed, so nothing re-applies it on its
   *  own — ask for that directly. */
  function cancelRecording(): void {
    recording = false;
    void applyGlobalHotkey();
  }

  /** Recording ends with a chord. Re-recording the one already stored leaves the
   *  store at the same value — a `writable` does not notify subscribers of a set
   *  that does not change anything — so the layout's subscription would never
   *  re-apply it and the hotkey would stay unregistered from `enterRecording`;
   *  apply directly in that case. A genuinely new chord persists as usual and the
   *  layout's subscription re-applies it; asking again here would just register
   *  the same accelerator twice. */
  function commitRecording(chord: string): void {
    recording = false;
    if (chord === get(globalHotkey)) void applyGlobalHotkey();
    else globalHotkey.set(chord);
  }

  function onKeydown(e: KeyboardEvent): void {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === 'Escape' && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
      cancelRecording();
      return;
    }
    const chord = chordFromEvent(e);
    if (!chord) return; // a lone modifier: keep listening
    const p = parseChord(chord);
    if (!p?.ctrl && !p?.alt && !p?.meta) return; // needs a real modifier: keep listening
    if (chordToAccelerator(chord) === null) return; // not a key the backend can register: keep listening
    commitRecording(chord);
  }

  function onPointerDown(e: PointerEvent): void {
    if (!recording) return;
    if (rowEl?.contains(e.target as Node)) return; // its own row handles this
    cancelRecording();
  }

  /** Switching to another app mid-recording leaves the window blurred with the
   *  hotkey unregistered until something re-applies it; treat it as a cancel. */
  function onWindowBlur(): void {
    if (!recording) return;
    cancelRecording();
  }

  // Leaving the page mid-recording (e.g. navigating away from Settings) must not
  // leave the hotkey unregistered behind it.
  $effect(() => () => {
    if (recording) void applyGlobalHotkey();
  });

  const btn =
    'rounded-full border border-default px-3 py-1 text-xs text-muted transition hover:border-strong ' +
    'hover:text-fg disabled:cursor-not-allowed disabled:opacity-40 focus-visible:outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus';
</script>

<svelte:window onkeydowncapture={onKeydown} onpointerdowncapture={onPointerDown} onblur={onWindowBlur} />

<div bind:this={rowEl}>
  <div class="flex items-center justify-between gap-4">
    <div class="min-w-0">
      <p class="text-sm">Show / hide OmnySSH</p>
      <p class="text-xs text-muted">
        A system-wide shortcut that brings OmnySSH forward or hides it. Ctrl+Space can clash with
        an input method's language switch; pick another if it does.
      </p>
    </div>
    <div class="flex items-center gap-2">
      <kbd
        class="min-w-[12rem] rounded-md bg-surface-inset px-2 py-1 text-center font-mono text-xs {recording
          ? 'ring-2 ring-focus'
          : ''}"
      >
        {recording ? 'Press keys… (Esc cancels)' : $globalHotkey ? formatChord($globalHotkey, isMac) : 'Off'}
      </kbd>
      <button
        type="button"
        class={btn}
        aria-pressed={recording}
        aria-label="Change show/hide shortcut"
        onclick={() => (recording ? cancelRecording() : enterRecording())}
      >
        {recording ? 'Cancel' : 'Change'}
      </button>
      <button
        type="button"
        class={btn}
        disabled={$globalHotkey === null}
        aria-label="Turn off the global hotkey"
        onclick={() => globalHotkey.set(null)}
      >
        Turn off
      </button>
      <button
        type="button"
        class={btn}
        disabled={$globalHotkey === DEFAULT_HOTKEY}
        aria-label="Reset the global hotkey"
        onclick={() => globalHotkey.reset()}
      >
        Reset
      </button>
    </div>
  </div>
  {#if $globalHotkeyError}
    <p role="alert" class="mt-1 text-right text-xs text-status-crit">{$globalHotkeyError}</p>
  {/if}
</div>
