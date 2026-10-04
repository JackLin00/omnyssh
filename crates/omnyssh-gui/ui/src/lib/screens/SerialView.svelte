<script lang="ts">
  // A serial port tab. `terminal` mode is interactive: keystrokes go to the port, with
  // Enter rewritten to what the device expects. `monitor` mode is receive-only, for
  // MCUs that only print. Either can flip between text and a hex dump; the flip
  // re-renders the kept history, so nothing already received is lost. Kept mounted
  // for the tab's whole life, like TerminalView.
  import '@xterm/xterm/css/xterm.css';
  import { onMount, onDestroy, untrack } from 'svelte';
  import { get } from 'svelte/store';
  import type { Terminal } from '@xterm/xterm';
  import type { FitAddon } from '@xterm/addon-fit';
  import type { SearchAddon } from '@xterm/addon-search';
  import { Channel } from '@tauri-apps/api/core';
  import { Button } from '$lib/theme';
  import { terminalColors } from '$lib/stores/terminalScheme';
  import { sessions, type Session } from '$lib/stores/sessions';
  import { lastError } from '$lib/stores/notifications';
  import { dialogs } from '$lib/stores/dialogs';
  import { serialOpen, serialWrite, serialClose } from '$lib/ipc/commands';
  import { attachMouseClipboard, copySelection, pasteClipboard } from './terminalClipboard';
  import { chunkBytes } from './terminalInput';
  import { matchTerminalAction, quickSlot } from './terminalShortcuts';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import { quickCommandBytes } from '$lib/stores/quickCommands';
  import { serialTimestamps, terminalFontSize } from '$lib/stores/terminalPrefs';
  import { ByteHistory, SerialFormatter, mapEnter, type SerialDisplay } from './serialFormat';
  import type { SerialExitDto, TerminalBytes } from '$lib/bindings';
  import TerminalSearch from './TerminalSearch.svelte';
  import { HIGHLIGHT_LIMIT } from './terminalSearch';
  import QuickCommandBar from './QuickCommandBar.svelte';
  import { attachWheelZoom } from './terminalZoom';

  let { session, active }: { session: Session; active: boolean } = $props();
  // SerialConnect always spawns serial tabs with their options, fixed for the tab's life.
  // svelte-ignore state_referenced_locally
  const opts = session.serial!;

  // Same stack as TerminalView (see the comment there for the ordering rule).
  const MONO =
    'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace, ' +
    '"Symbols Nerd Font Mono", "Symbols Nerd Font", "MesloLGS NF", ' +
    '"JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", ' +
    '"Hack Nerd Font Mono", "Hack Nerd Font", ' +
    '"FiraCode Nerd Font Mono", "FiraCode Nerd Font"';
  /** Received bytes kept for a text/hex re-render. */
  const HISTORY_LIMIT = 2 * 1024 * 1024;
  /** Characters per xterm write when replaying history, so the UI can paint between writes. */
  const REPLAY_BATCH = 64 * 1024;
  const DISPLAYS: SerialDisplay[] = ['text', 'hex'];
  const ENCODER = new TextEncoder();

  let container: HTMLDivElement;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let searchAddon = $state<SearchAddon>();
  let searchOpen = $state(false);
  let searchFocus = $state(0);
  let serialId: number | undefined;
  let destroyed = false;
  let failed = false;
  let ready = $state(false);
  let canSend = $state(false);
  let display = $state<SerialDisplay>('text');
  // Stamps are for the receive-only monitor's text display; hex stays a plain dump.
  const stamped = $derived(opts.mode === 'monitor' && display === 'text' && $serialTimestamps);
  // Only the initial values: the formatter is a plain stateful object, not reactive, and
  // later changes go through `rerender()`'s `formatter.reset(display, stamped)`.
  // svelte-ignore state_referenced_locally
  const formatter = new SerialFormatter(display, stamped);
  const history = new ByteHistory(HISTORY_LIMIT);
  let themeUnsub: (() => void) | undefined;
  let fontSizeUnsub: (() => void) | undefined;
  let mouseClipboardOff: (() => void) | undefined;
  let zoomOff: (() => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitScheduled = false;
  // The small "<n> px" badge shown in this tab when Ctrl+wheel changes the font size.
  let sizeBadge = $state<number | null>(null);
  let badgeTimer: ReturnType<typeof setTimeout> | undefined;
  function flashSize(): void {
    sizeBadge = get(terminalFontSize);
    clearTimeout(badgeTimer);
    badgeTimer = setTimeout(() => {
      sizeBadge = null;
    }, 1000);
  }

  // Chunked and serialized like TerminalView's input, so a large paste stays in order.
  let writeChain: Promise<void> = Promise.resolve();
  function sendInput(bytes: Uint8Array): void {
    if (serialId == null || bytes.length === 0) return;
    writeChain = writeChain.then(async () => {
      for (const chunk of chunkBytes(bytes)) {
        if (destroyed || serialId == null) return;
        try {
          await serialWrite(serialId, Array.from(chunk));
        } catch {
          return;
        }
      }
    });
  }

  /** Re-render the kept history into `term`, from scratch. Called after the display or
   *  timestamp toggle changes. A no-op before `term` exists (e.g. the initial mount). */
  function rerender(): void {
    if (!term) return;
    formatter.reset(display, stamped);
    // In-band RIS, not term.reset(): it is ordered after output xterm has queued but
    // not parsed yet, so nothing from before the flip lands after it.
    term.write('\x1bc');
    let batch = '';
    for (const e of history.entries()) {
      batch += formatter.push(e.bytes, e.at);
      if (batch.length >= REPLAY_BATCH) {
        term.write(batch);
        batch = '';
      }
    }
    if (batch) term.write(batch);
  }

  // The effect below re-renders once the display changes.
  function setDisplay(next: SerialDisplay): void {
    display = next;
  }

  function clear(): void {
    history.clear();
    formatter.reset(display, stamped);
    term?.write('\x1bc');
  }

  function openSearch(): void {
    searchOpen = true;
    searchFocus += 1;
  }

  function closeSearch(): void {
    searchOpen = false;
    term?.focus();
  }

  /** Fit only while visible: a hidden container measures 0. A serial line has no
   *  window size, so nothing is sent to the device. */
  function safeFit(): void {
    if (!term || !fitAddon || !active) return;
    try {
      fitAddon.fit();
    } catch {
      // Not laid out yet; the next resize refits.
    }
  }

  function scheduleFit(): void {
    if (fitScheduled) return;
    fitScheduled = true;
    requestAnimationFrame(() => {
      fitScheduled = false;
      safeFit();
    });
  }

  onMount(() => {
    void (async () => {
      const [{ Terminal }, { FitAddon }, searchModule] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
        import('@xterm/addon-search')
      ]);
      if (destroyed) return;

      let lastFontSize = get(terminalFontSize);
      term = new Terminal({
        fontFamily: MONO,
        fontSize: lastFontSize,
        scrollback: 10000,
        // MCUs print bare LF line ends; without this every line would stair-step.
        convertEol: true,
        cursorBlink: opts.mode === 'terminal',
        disableStdin: opts.mode === 'monitor',
        // 3, not xterm's higher presets: it rescues truly unreadable pairs a program
        // prints while leaving each scheme's own published colours alone -- 4.5 visibly
        // shifted Solarized Light's and Gruvbox's accent colours off their real hues.
        minimumContrastRatio: 3,
        allowProposedApi: true // the search addon's match highlights are decorations
      });
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      const { SearchAddon } = searchModule;
      searchAddon = new SearchAddon({ highlightLimit: HIGHLIGHT_LIMIT });
      term.loadAddon(searchAddon);
      term.open(container);
      // Receive-only monitor tabs send nothing, so right-click keeps its native menu.
      mouseClipboardOff = attachMouseClipboard(term, container, opts.mode === 'terminal');
      // Ctrl+wheel over the terminal steps the shared font size; swallowed in the
      // capture phase so the webview never page-zooms (see terminalZoom.ts).
      zoomOff = attachWheelZoom(container, (d) => {
        terminalFontSize.step(d);
        flashSize();
      });
      // Copy and find (both modes) and paste (terminal mode) follow Settings → Keyboard
      // shortcuts; the pane chords mean nothing in a serial tab, so those keys go to the
      // device. A held paste chord repeats keydown with no keyup between; only the first
      // press should paste (copy and find stay on every repeat — copying the same
      // selection, or reopening/reselecting the find bar, is harmless).
      term.attachCustomKeyEventHandler((e) => {
        const action = matchTerminalAction(e, get(terminalShortcuts));
        const slot = action ? quickSlot(action) : null;
        // An empty command bar slot takes no key of its own: let it reach the device.
        if (slot !== null && !quickCommandBytes(slot)?.length) return true;
        const handled =
          action === 'copy' ||
          action === 'find' ||
          (action === 'paste' && opts.mode === 'terminal') ||
          slot !== null;
        if (!handled) return true;
        e.preventDefault();
        if (e.type === 'keydown' && term) {
          if (action === 'copy') copySelection(term);
          else if (action === 'find') openSearch();
          else if (slot !== null) {
            if (!e.repeat) {
              const bytes = quickCommandBytes(slot);
              if (bytes && bytes.length > 0) sendInput(bytes);
            }
          } else if (!e.repeat) void pasteClipboard(term);
        }
        return false;
      });
      themeUnsub = terminalColors.subscribe((c) => {
        if (term) term.options.theme = c;
      });
      // Every terminal shares one font size: apply it live and refit, skipping the
      // initial fire that matches what the terminal was already created with.
      fontSizeUnsub = terminalFontSize.subscribe((n) => {
        if (term && n !== lastFontSize) {
          term.options.fontSize = n;
          scheduleFit();
        }
        lastFontSize = n;
      });

      // The raw path delivers an ArrayBuffer despite the `number[]` type (see TerminalView).
      const output = new Channel<TerminalBytes>();
      output.onmessage = (msg) => {
        const bytes = new Uint8Array(msg as unknown as ArrayBuffer);
        const at = Date.now();
        history.push(bytes, at);
        term?.write(formatter.push(bytes, at));
      };
      const exit = new Channel<SerialExitDto>();
      exit.onmessage = (msg) => {
        if (destroyed) return;
        // Keep the tab and its output: the log up to the failure is what matters.
        failed = true;
        canSend = false;
        sessions.setStatus(session.id, 'failed');
        lastError.set(`${opts.config.port}: ${msg.error}`);
      };

      safeFit();
      const id = await serialOpen(opts.config, output, exit);
      if (destroyed) {
        void serialClose(id).catch(() => {});
        return;
      }
      serialId = id;
      sessions.setTermId(session.id, id);
      if (!failed) {
        sessions.setStatus(session.id, 'connected');
        canSend = true;
      }

      if (opts.mode === 'terminal') {
        term.onData((data) => sendInput(ENCODER.encode(mapEnter(data, opts.enter))));
        // onBinary carries raw 8-bit sequences (e.g. legacy mouse reporting) that must go
        // byte-for-byte, not re-encoded, and never Enter-mapped.
        term.onBinary((data) =>
          sendInput(Uint8Array.from(data, (ch) => ch.charCodeAt(0) & 0xff))
        );
      }

      resizeObserver = new ResizeObserver(() => scheduleFit());
      resizeObserver.observe(container);

      ready = true;
      if (active && get(dialogs).length === 0) term.focus();
    })().catch((err) => {
      // A tab closed while its open was in flight has nothing left to report to.
      if (destroyed) return;
      // The port could not be opened (missing, or held by another program).
      lastError.set(err instanceof Error ? err.message : String(err));
      sessions.setStatus(session.id, 'failed');
      canSend = false;
    });
  });

  onDestroy(() => {
    destroyed = true;
    themeUnsub?.();
    fontSizeUnsub?.();
    mouseClipboardOff?.();
    zoomOff?.();
    clearTimeout(badgeTimer);
    resizeObserver?.disconnect();
    if (serialId != null) void serialClose(serialId).catch(() => {});
    term?.dispose();
    term = undefined;
  });

  $effect(() => {
    if (active && ready) {
      const free = $dialogs.length === 0;
      requestAnimationFrame(() => {
        safeFit();
        if (free) term?.focus();
      });
    }
  });

  // Re-render when the display or the timestamp toggle changes — once per change, since
  // this is the only caller. A no-op on the initial run: `term` doesn't exist yet (it's
  // created asynchronously in onMount). The render itself is untracked.
  $effect(() => {
    void display;
    void stamped;
    untrack(rerender);
  });
</script>

<div
  class="absolute inset-0 flex-col overflow-hidden bg-surface pb-4 pt-[var(--titlebar-h)] {active
    ? 'flex'
    : 'hidden'}"
>
  <div class="flex shrink-0 items-center gap-3 border-b border-default px-3 py-2 text-xs">
    <span class="font-mono text-fg">{session.hostName}</span>
    <span class="text-faint">{opts.mode === 'monitor' ? 'receive only' : 'terminal'}</span>
    <span class="flex-1"></span>
    <div class="flex rounded-full border border-default p-0.5" role="group" aria-label="Display">
      {#each DISPLAYS as d (d)}
        <button
          type="button"
          class="rounded-full px-3 py-0.5 transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {display ===
          d
            ? 'bg-accent text-accent-fg'
            : 'text-muted hover:text-fg'}"
          aria-pressed={display === d}
          onclick={() => setDisplay(d)}
        >
          {d === 'text' ? 'Text' : 'HEX'}
        </button>
      {/each}
    </div>
    {#if opts.mode === 'monitor'}
      <button
        type="button"
        class="rounded-full border border-default px-3 py-0.5 text-xs transition focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {$serialTimestamps
          ? 'bg-accent text-accent-fg'
          : 'text-muted hover:text-fg'} disabled:cursor-not-allowed disabled:opacity-40"
        aria-pressed={$serialTimestamps}
        disabled={display === 'hex'}
        title={display === 'hex' ? 'Timestamps show in the text display' : 'Stamp each line with when it arrived'}
        onclick={() => serialTimestamps.toggle()}
      >
        Time
      </button>
    {/if}
    <Button variant="ghost" title="Clear the output" onclick={clear}>Clear</Button>
  </div>
  <div class="relative min-h-0 flex-1 px-2 pt-2" style="background: {$terminalColors.background}">
    <div bind:this={container} class="h-full w-full"></div>
    {#if searchOpen && searchAddon}
      <div class="absolute right-3 top-2 z-20 max-w-[calc(100%-1.5rem)]">
        <TerminalSearch addon={searchAddon} focusToken={searchFocus} onClose={closeSearch} />
      </div>
    {/if}
    {#if sizeBadge !== null}
      <div
        class="pointer-events-none absolute left-1/2 top-1/2 z-30 -translate-x-1/2 -translate-y-1/2 rounded-md bg-surface-raised px-3 py-1.5 text-sm font-medium text-fg shadow-soft"
        aria-live="polite"
      >
        {sizeBadge} px
      </div>
    {/if}
  </div>
  <QuickCommandBar
    enabled={canSend}
    onSend={(b) => {
      sendInput(b);
      // A bar button leaves DOM focus on itself; give the keyboard back to the
      // terminal, the same place a click into it would.
      term?.focus();
    }}
  />
</div>
