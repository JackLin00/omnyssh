<script lang="ts">
  // One terminal pane: an xterm and the SSH session behind it (tech-gui.md §3.1). A
  // terminal tab (TerminalView) lays one or more out as split panes, each its own
  // session to the tab's host. Kept mounted for the pane's whole life — hidden with its
  // tab, not destroyed — so scrollback and the byte stream survive tab switches. Raw
  // output arrives on a per-session channel (§3.3/§3.6); keystrokes/resizes go back over
  // the terminal commands. Subscribes to the theme store and re-themes live (§5.1).
  import '@xterm/xterm/css/xterm.css';
  import { onMount, onDestroy } from 'svelte';
  import { get } from 'svelte/store';
  import type { Terminal } from '@xterm/xterm';
  import type { FitAddon } from '@xterm/addon-fit';
  import type { SearchAddon } from '@xterm/addon-search';
  import { Channel } from '@tauri-apps/api/core';
  import { Icon } from '$lib/theme';
  import { theme } from '$lib/stores/theme';
  import { xtermTheme } from '$lib/theme/terminalTheme';
  import type { SessionStatus } from '$lib/stores/sessions';
  import { registerPaneExit } from '$lib/stores/paneExits';
  import { terminalDidExit } from '$lib/ipc/router';
  import { lastError } from '$lib/stores/notifications';
  import { dialogs } from '$lib/stores/dialogs';
  import {
    terminalOpen,
    terminalWrite,
    terminalResize,
    terminalClose,
    terminalPaste
  } from '$lib/ipc/commands';
  import { shouldFadeTop } from './terminalFade';
  import { chunkBytes, layoutFallback } from './terminalInput';
  import { attachMouseClipboard, copySelection, pasteClipboard } from './terminalClipboard';
  import { matchTerminalAction, type TerminalAction } from './terminalShortcuts';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import type { SplitDir } from './splitLayout';
  import type { TerminalBytes } from '$lib/bindings';
  import TerminalSearch from './TerminalSearch.svelte';
  import { HIGHLIGHT_LIMIT } from './terminalSearch';

  let {
    hostName,
    active,
    focused,
    framed,
    onStatus,
    onFocus,
    onSplit,
    onClose
  }: {
    hostName: string;
    /** Its tab is the visible one. */
    active: boolean;
    /** The pane that takes the keyboard within its tab. */
    focused: boolean;
    /** Draw the focus frame: only once the tab has more than one pane. */
    framed: boolean;
    onStatus: (status: SessionStatus) => void;
    onFocus: () => void;
    onSplit: (dir: SplitDir) => void;
    /** The user closed the pane, or its session ended. */
    onClose: () => void;
  } = $props();

  // The Nerd Font families come after the generic `monospace`, not merely after the
  // named system ones: the named list is macOS/Windows-only, so on a Linux desktop a
  // patched font ahead of the generic would become the terminal's Latin face and size
  // its cell from itself. Per-character fallback continues past a generic family, so the
  // Private Use Area glyphs (starship, powerlevel10k, eza --icons) still reach the tail.
  // Within the tail, the single-width variants come first — Nerd Fonts v3 ships icons at
  // double width in the bare family and one cell wide in its `Mono` twin.
  const MONO =
    'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace, ' +
    '"Symbols Nerd Font Mono", "Symbols Nerd Font", "MesloLGS NF", ' +
    '"JetBrainsMono Nerd Font Mono", "JetBrainsMono Nerd Font", ' +
    '"Hack Nerd Font Mono", "Hack Nerd Font", ' +
    '"FiraCode Nerd Font Mono", "FiraCode Nerd Font"';
  // One encoder for the keystroke hot path instead of one per input event.
  const ENCODER = new TextEncoder();

  // A large paste arrives as one onData; sending it as a single number[] would freeze
  // the UI thread (§9). Split into bounded chunks and await each so paint yields between
  // them; a serialization chain keeps all input strictly in order across events.
  let writeChain: Promise<void> = Promise.resolve();
  function sendInput(bytes: Uint8Array): void {
    if (termId == null || bytes.length === 0) return;
    writeChain = writeChain.then(async () => {
      for (const chunk of chunkBytes(bytes)) {
        if (destroyed || termId == null) return;
        try {
          await terminalWrite(termId, Array.from(chunk));
        } catch {
          // Stop this input on a write failure rather than sending a gapped stream.
          return;
        }
      }
    });
  }

  function runAction(action: TerminalAction): void {
    if (!term) return;
    if (action === 'copy') copySelection(term);
    else if (action === 'paste') void pasteClipboard(term);
    else if (action === 'find') openSearch();
    else if (action === 'closePane') onClose();
    else onSplit(action === 'splitRight' ? 'row' : 'column');
  }

  function openSearch(): void {
    searchOpen = true;
    searchFocus += 1;
  }

  function closeSearch(): void {
    searchOpen = false;
    term?.focus();
  }

  let container: HTMLDivElement;
  let toolbar: HTMLDivElement;
  // Read inside the focus $effect below, so Svelte needs it reactive to track that read.
  let searchBox = $state<HTMLDivElement>();
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let searchAddon = $state<SearchAddon>();
  let searchOpen = $state(false);
  let searchFocus = $state(0);
  let termId: number | undefined;
  let destroyed = false;
  let connected = false;
  let ready = $state(false);
  let themeUnsub: (() => void) | undefined;
  let mouseClipboardOff: (() => void) | undefined;
  let exitOff: (() => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitScheduled = false;
  // The top-edge fade dissolves scrolled output into the top edge, but never the live
  // prompt: after `clear`/Ctrl+L the cursor homes to the top, so the fade must lift
  // there (see terminalFade). Recomputed after every write too, since those resets
  // move the viewport without firing onScroll.
  let scrolled = $state(false);
  function syncScrolled(): void {
    const buf = term?.buffer.active;
    scrolled = !!buf && shouldFadeTop(buf.viewportY, buf.baseY, buf.cursorY);
  }

  /** Fit the terminal to its container and tell the backend, but only while visible —
   *  a hidden (display:none) container measures 0, so it refits when shown instead. */
  function safeFit(): void {
    if (!term || !fitAddon || !active) return;
    try {
      fitAddon.fit();
    } catch {
      return;
    }
    if (termId != null) void terminalResize(termId, term.cols, term.rows).catch(() => {});
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
    onStatus('connecting');
    void (async () => {
      const [{ Terminal }, { FitAddon }, searchModule] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
        import('@xterm/addon-search')
      ]);
      if (destroyed) return;

      term = new Terminal({
        fontFamily: MONO,
        fontSize: 13,
        cursorBlink: true,
        scrollback: 5000,
        allowProposedApi: true // the search addon's match highlights are decorations
      });
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      const { SearchAddon } = searchModule;
      searchAddon = new SearchAddon({ highlightLimit: HIGHLIGHT_LIMIT });
      term.loadAddon(searchAddon);
      term.open(container);
      mouseClipboardOff = attachMouseClipboard(term, container, true);
      term.onScroll(syncScrolled);

      // The #1 theme-regression guard (§5.1): push the matching xterm theme to this
      // terminal — including already-open ones — whenever the store flips. Its
      // synchronous first call (pre-paint) also sets the initial theme.
      themeUnsub = theme.subscribe((t) => {
        if (term) term.options.theme = xtermTheme(t);
      });

      // Route raw output into xterm. The channel is typed `number[]`, but the raw path
      // actually delivers an `ArrayBuffer` (§3.3); `Uint8Array` wraps either.
      const channel = new Channel<TerminalBytes>();
      channel.onmessage = (msg) => {
        if (!term) return;
        if (!connected) {
          connected = true;
          onStatus('connected');
        }
        term.write(new Uint8Array(msg as unknown as ArrayBuffer), syncScrolled);
      };

      // Fit before opening so the remote PTY starts at the visible size.
      safeFit();
      const id = await terminalOpen(hostName, term.cols || 80, term.rows || 24, channel);
      if (destroyed) {
        void terminalClose(id).catch(() => {});
        return;
      }
      termId = id;
      // The remote may have already exited before this id was recorded (fast-fail
      // connect race): terminal-exited couldn't reach the pane, so close it now.
      if (terminalDidExit(id)) {
        onClose();
        return;
      }
      exitOff = registerPaneExit(id, () => onClose());

      term.attachCustomKeyEventHandler((e) => {
        // The chords from Settings → Keyboard shortcuts never reach the shell; they act
        // on keydown only. The copy runs inside the keydown, which WebKit requires. A
        // held chord repeats keydown with no keyup between; splitting, closing or
        // pasting again on every repeat would be surprising, so only copy and find also
        // act on a repeat (copying the same selection, or reopening/reselecting the
        // find bar, is harmless).
        const action = matchTerminalAction(e, get(terminalShortcuts));
        if (action) {
          e.preventDefault();
          if (e.type === 'keydown' && (action === 'copy' || action === 'find' || !e.repeat)) {
            runAction(action);
          }
          return false;
        }
        // Under a non-Latin layout WebKitGTK names no key; the physical one stands in.
        const fallback = layoutFallback(e);
        if (!fallback) return true;
        // As xterm does with a key it handles: nothing else acts on it.
        e.preventDefault();
        e.stopPropagation();
        if (fallback.kind === 'control') {
          term?.input(fallback.data);
        } else {
          terminalPaste().catch((err) => {
            lastError.set(`Paste failed: ${err instanceof Error ? err.message : String(err)}`);
          });
        }
        return false;
      });
      // Text keystrokes/paste are UTF-8; onBinary carries raw 8-bit sequences
      // (e.g. legacy mouse reporting) that must go byte-for-byte, not re-encoded.
      term.onData((data) => sendInput(ENCODER.encode(data)));
      term.onBinary((data) => sendInput(Uint8Array.from(data, (ch) => ch.charCodeAt(0) & 0xff)));

      resizeObserver = new ResizeObserver(() => scheduleFit());
      resizeObserver.observe(container);

      ready = true;
      if (active && focused && get(dialogs).length === 0) term.focus();
    })().catch((err) => {
      // `terminal_open` itself failed (e.g. the session could not be spawned): no
      // PtyExited follows, so mark the pane failed here instead of leaving it hung.
      if (destroyed) return;
      lastError.set(err instanceof Error ? err.message : String(err));
      onStatus('failed');
    });
  });

  onDestroy(() => {
    destroyed = true;
    themeUnsub?.();
    mouseClipboardOff?.();
    exitOff?.();
    resizeObserver?.disconnect();
    // Idempotent: a remote-exit teardown already dropped this id backend-side (§3.4).
    if (termId != null) void terminalClose(termId).catch(() => {});
    term?.dispose();
    // Null it so a byte still in flight (destroyed-before-open race) can't write to
    // a disposed terminal — the channel callback's `if (!term)` guard then bails.
    term = undefined;
  });

  // Becoming visible: a hidden container measured 0, so refit, and the focused pane
  // takes the keyboard. An open dialog keeps it (keystrokes meant for a key passphrase
  // must never reach the shell); the pane takes it back once the last one closes. Never
  // while the find bar holds focus — clicking another pane and back must not pull the
  // keyboard out of an open find input.
  $effect(() => {
    if (active && ready) {
      const take = focused && $dialogs.length === 0;
      requestAnimationFrame(() => {
        safeFit();
        if (take && !searchBox?.contains(document.activeElement)) term?.focus();
        syncScrolled();
      });
    }
  });

  // Focus landing on a toolbar button (Tab) or the find bar leaves the pane's focus
  // alone: making the pane focused would hand the keyboard to xterm next frame and pull
  // it off the button or input. A click still focuses the pane through pointerdown.
  function focusIn(e: FocusEvent): void {
    if (toolbar?.contains(e.target as Node) || searchBox?.contains(e.target as Node)) return;
    onFocus();
  }

  const toolBtn =
    'grid h-6 w-6 place-items-center rounded bg-surface-inset text-muted transition ' +
    'hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

<!-- Clicking anywhere in the pane, or xterm's textarea taking focus, makes it the
     tab's focused pane. -->
<div
  class="group relative h-full w-full {framed ? 'p-0.5' : ''}"
  role="presentation"
  onpointerdown={onFocus}
  onfocusin={focusIn}
>
  <div
    class="h-full w-full rounded {framed && focused ? 'ring-1 ring-focus' : ''}"
  >
    <div bind:this={container} class="h-full w-full" class:term-fade={scrolled}></div>
  </div>
  <div
    bind:this={toolbar}
    class="absolute right-1.5 top-1.5 z-10 flex gap-1 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100"
  >
    <button type="button" class={toolBtn} title="Split right (Alt+Shift+=)" aria-label="Split right" onclick={() => onSplit('row')}>
      <Icon name="splitRight" size={14} />
    </button>
    <button type="button" class={toolBtn} title="Split down (Alt+Shift+-)" aria-label="Split down" onclick={() => onSplit('column')}>
      <Icon name="splitDown" size={14} />
    </button>
    <button type="button" class={toolBtn} title="Close pane (Ctrl+Shift+W)" aria-label="Close pane" onclick={onClose}>
      <Icon name="close" size={14} />
    </button>
  </div>
  {#if searchOpen && searchAddon}
    <div bind:this={searchBox} class="absolute right-1.5 top-9 z-20 max-w-[calc(100%-0.75rem)]">
      <TerminalSearch addon={searchAddon} focusToken={searchFocus} onClose={closeSearch} />
    </div>
  {/if}
</div>

<style>
  /* Scrolled output dissolves into the top edge instead of hard-clipping (on only while
     scrolled, so the first line stays crisp). black/transparent are mask alphas. */
  .term-fade {
    -webkit-mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
    mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
  }
</style>
