<script lang="ts">
  // A serial port tab. `terminal` mode is interactive: keystrokes go to the port, with
  // Enter rewritten to what the device expects. `monitor` mode is receive-only, for
  // MCUs that only print. Either can flip between text and a hex dump; the flip
  // re-renders the kept history, so nothing already received is lost. Kept mounted
  // for the tab's whole life, like TerminalView.
  import '@xterm/xterm/css/xterm.css';
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Terminal } from '@xterm/xterm';
  import type { FitAddon } from '@xterm/addon-fit';
  import { Channel } from '@tauri-apps/api/core';
  import { Button } from '$lib/theme';
  import { theme } from '$lib/stores/theme';
  import { xtermTheme } from '$lib/theme/terminalTheme';
  import { sessions, type Session } from '$lib/stores/sessions';
  import { lastError } from '$lib/stores/notifications';
  import { dialogs } from '$lib/stores/dialogs';
  import { serialOpen, serialWrite, serialClose } from '$lib/ipc/commands';
  import { chunkBytes, isCopyShortcut } from './terminalInput';
  import { isMac } from '$lib/platform';
  import { ByteHistory, SerialFormatter, mapEnter, type SerialDisplay } from './serialFormat';
  import type { SerialExitDto, TerminalBytes } from '$lib/bindings';

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
  let serialId: number | undefined;
  let destroyed = false;
  let failed = false;
  let ready = $state(false);
  let display = $state<SerialDisplay>('text');
  const formatter = new SerialFormatter('text');
  const history = new ByteHistory(HISTORY_LIMIT);
  let themeUnsub: (() => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitScheduled = false;

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

  function setDisplay(next: SerialDisplay): void {
    if (!term || next === display) return;
    display = next;
    formatter.reset(next);
    // In-band RIS, not term.reset(): it is ordered after output xterm has queued but
    // not parsed yet, so nothing from before the flip lands after it.
    term.write('\x1bc');
    let batch = '';
    for (const chunk of history.all()) {
      batch += formatter.push(chunk);
      if (batch.length >= REPLAY_BATCH) {
        term.write(batch);
        batch = '';
      }
    }
    if (batch) term.write(batch);
  }

  function clear(): void {
    history.clear();
    formatter.reset(display);
    term?.write('\x1bc');
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
      const [{ Terminal }, { FitAddon }] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit')
      ]);
      if (destroyed) return;

      term = new Terminal({
        fontFamily: MONO,
        fontSize: 13,
        scrollback: 10000,
        // MCUs print bare LF line ends; without this every line would stair-step.
        convertEol: true,
        cursorBlink: opts.mode === 'terminal',
        disableStdin: opts.mode === 'monitor'
      });
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.open(container);
      // Copy takes Ctrl+Shift+C whether or not anything is selected, in both modes, so
      // the chord never reaches the device. As in TerminalView.
      term.attachCustomKeyEventHandler((e) => {
        if (!isCopyShortcut(e, isMac)) return true;
        e.preventDefault();
        if (term?.hasSelection()) {
          navigator.clipboard.writeText(term.getSelection()).catch((err) => {
            lastError.set(`Copy failed: ${err instanceof Error ? err.message : String(err)}`);
          });
        }
        return false;
      });
      themeUnsub = theme.subscribe((t) => {
        if (term) term.options.theme = xtermTheme(t);
      });

      // The raw path delivers an ArrayBuffer despite the `number[]` type (see TerminalView).
      const output = new Channel<TerminalBytes>();
      output.onmessage = (msg) => {
        const bytes = new Uint8Array(msg as unknown as ArrayBuffer);
        history.push(bytes);
        term?.write(formatter.push(bytes));
      };
      const exit = new Channel<SerialExitDto>();
      exit.onmessage = (msg) => {
        if (destroyed) return;
        // Keep the tab and its output: the log up to the failure is what matters.
        failed = true;
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
      if (!failed) sessions.setStatus(session.id, 'connected');

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
    });
  });

  onDestroy(() => {
    destroyed = true;
    themeUnsub?.();
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
</script>

<div
  class="absolute inset-0 flex-col overflow-hidden bg-surface pt-[var(--titlebar-h)] {active
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
    <Button variant="ghost" title="Clear the output" onclick={clear}>Clear</Button>
  </div>
  <div class="min-h-0 flex-1 px-2 pb-4 pt-2">
    <div bind:this={container} class="h-full w-full"></div>
  </div>
</div>
