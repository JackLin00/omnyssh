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
  import { Icon, StatusDot } from '$lib/theme';
  import { terminalColors } from '$lib/stores/terminalScheme';
  import { sessionStatusDot, type SessionStatus } from '$lib/stores/sessions';
  import { registerPaneExit } from '$lib/stores/paneExits';
  import { terminalDidExit } from '$lib/ipc/router';
  import { lastError } from '$lib/stores/notifications';
  import { dialogs } from '$lib/stores/dialogs';
  import { hosts } from '$lib/stores/hosts';
  import { sessionLogs, startSessionLog, exportText } from '$lib/stores/sessionLogs';
  import {
    terminalOpen,
    terminalWrite,
    terminalResize,
    terminalClose,
    terminalPaste,
    logStop,
    logStatus
  } from '$lib/ipc/commands';
  import { shouldFadeTop } from './terminalFade';
  import { chunkBytes, layoutFallback } from './terminalInput';
  import { attachMouseClipboard, copySelection, pasteClipboard } from './terminalClipboard';
  import { matchTerminalAction, formatChord, quickSlot, type TerminalAction } from './terminalShortcuts';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import { quickCommandBytes } from '$lib/stores/quickCommands';
  import { logTimestamps, terminalFontSize } from '$lib/stores/terminalPrefs';
  import { attachWheelZoom } from './terminalZoom';
  import { isMac } from '$lib/platform';
  import type { SplitDir } from './splitLayout';
  import type { TerminalBytes } from '$lib/bindings';
  import TerminalSearch from './TerminalSearch.svelte';
  import { HIGHLIGHT_LIMIT } from './terminalSearch';
  import { bufferText, formatBytes, sshLogDetail } from './sessionLog';

  let {
    hostName,
    active,
    focused,
    framed,
    titled,
    status,
    onStatus,
    onFocus,
    onSplit,
    onClose,
    onMoveStart
  }: {
    hostName: string;
    /** Its tab is the visible one. */
    active: boolean;
    /** The pane that takes the keyboard within its tab. */
    focused: boolean;
    /** Draw the focus frame: only once the tab has more than one pane. */
    framed: boolean;
    /** Show the title bar: once the tab has more than one pane. */
    titled: boolean;
    /** This pane's connection state, for the title bar's dot. */
    status: SessionStatus;
    onStatus: (status: SessionStatus) => void;
    onFocus: () => void;
    onSplit: (dir: SplitDir) => void;
    /** The user closed the pane, or its session ended. */
    onClose: () => void;
    /** A press on the title bar's free area: the tab may start moving this pane. */
    onMoveStart: (e: PointerEvent) => void;
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

  /** Send bytes to this pane's shell, in order with typed input (the command bar). */
  export function sendBytes(bytes: Uint8Array): void {
    sendInput(bytes);
  }

  /** Give this pane's xterm the keyboard back (after a command bar button, which would
   *  otherwise leave focus parked on the button itself). */
  export function focusTerminal(): void {
    term?.focus();
  }

  function runAction(action: TerminalAction): void {
    if (!term) return;
    const slot = quickSlot(action);
    if (slot !== null) {
      const bytes = quickCommandBytes(slot);
      if (bytes && bytes.length > 0) sendInput(bytes);
      return;
    }
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
  let toolbar = $state<HTMLDivElement>();
  let titleBar = $state<HTMLDivElement>();
  // Read inside the focus $effect below, so Svelte needs it reactive to track that read.
  let searchBox = $state<HTMLDivElement>();
  /** The shell's own title (user@host: dir), when it has set one. */
  let title = $state('');
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let searchAddon = $state<SearchAddon>();
  let searchOpen = $state(false);
  let searchFocus = $state(0);
  // Reactive for the log buttons, which wait for it.
  let termId = $state<number>();
  let destroyed = false;
  let connected = false;
  let ready = $state(false);
  let themeUnsub: (() => void) | undefined;
  let fontSizeUnsub: (() => void) | undefined;
  let mouseClipboardOff: (() => void) | undefined;
  let zoomOff: (() => void) | undefined;
  let exitOff: (() => void) | undefined;
  let resizeObserver: ResizeObserver | undefined;
  let fitScheduled = false;
  // The small "<n> px" badge shown in this pane when Ctrl+wheel changes the font size.
  let sizeBadge = $state<number | null>(null);
  let badgeTimer: ReturnType<typeof setTimeout> | undefined;
  function flashSize(): void {
    sizeBadge = get(terminalFontSize);
    clearTimeout(badgeTimer);
    badgeTimer = setTimeout(() => {
      sizeBadge = null;
    }, 1000);
  }
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

      let lastFontSize = get(terminalFontSize);
      term = new Terminal({
        fontFamily: MONO,
        fontSize: lastFontSize,
        cursorBlink: true,
        scrollback: 5000,
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
      mouseClipboardOff = attachMouseClipboard(term, container, true);
      // Ctrl+wheel over the terminal steps the shared font size; swallowed in the
      // capture phase so the webview never page-zooms (see terminalZoom.ts).
      zoomOff = attachWheelZoom(container, (d) => {
        terminalFontSize.step(d);
        flashSize();
      });
      term.onScroll(syncScrolled);
      // The shell's own title (user@host: dir) names the pane when it sets one.
      term.onTitleChange((t) => (title = t));

      // The #1 theme-regression guard (§5.1): push the matching xterm theme to this
      // terminal — including already-open ones — whenever the chosen scheme or the app
      // theme flips. Its synchronous first call (pre-paint) also sets the initial theme.
      themeUnsub = terminalColors.subscribe((c) => {
        if (term) term.options.theme = c;
      });
      // Every terminal shares one font size: apply it live and refit (which also tells
      // the backend to resize the PTY), skipping the initial fire that matches what the
      // terminal was already created with.
      fontSizeUnsub = terminalFontSize.subscribe((n) => {
        if (term && n !== lastFontSize) {
          term.options.fontSize = n;
          scheduleFit();
        }
        lastFontSize = n;
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
          // An empty command bar slot takes no key of its own: let Alt+N reach the
          // shell (xterm's own Alt handling) rather than swallowing it for nothing.
          const slot = quickSlot(action);
          if (slot !== null && !quickCommandBytes(slot)?.length) return true;
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
    fontSizeUnsub?.();
    mouseClipboardOff?.();
    zoomOff?.();
    clearTimeout(badgeTimer);
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
  // while the find bar holds focus, or Tab has landed on a title-bar button — clicking
  // another pane and back, or tabbing onto a button, must not pull the keyboard away.
  // A press elsewhere on the title bar's free area never moves DOM focus there (see the
  // title bar's own onmousedown below), so this is not also excluded.
  $effect(() => {
    if (active && ready) {
      const take = focused && $dialogs.length === 0;
      requestAnimationFrame(() => {
        safeFit();
        const focusedEl = document.activeElement;
        const onTitleBarButton = !!titleBar?.contains(focusedEl) && focusedEl !== titleBar;
        if (take && !searchBox?.contains(focusedEl) && !onTitleBarButton) {
          term?.focus();
        }
        syncScrolled();
      });
    }
  });

  // Focus landing on a toolbar or title bar button (Tab) or the find bar leaves the
  // pane's focus alone: making the pane focused would hand the keyboard to xterm next
  // frame and pull it off the button or input. A click still focuses the pane through
  // pointerdown.
  function focusIn(e: FocusEvent): void {
    if (
      toolbar?.contains(e.target as Node) ||
      titleBar?.contains(e.target as Node) ||
      searchBox?.contains(e.target as Node)
    ) {
      return;
    }
    onFocus();
  }

  // The toolbar/title-bar backgrounds and text are tinted from the terminal scheme
  // (--term-bg/--term-fg set by TerminalView), not the app's own surface tokens, so the
  // pane's chrome reads as part of the terminal rather than the app shell -- including on
  // a dark-only scheme (Nord, Dracula) while the app theme itself is light, where the
  // app's own `text-muted`/`hover:bg-surface` tokens would be unreadable against it. The
  // shared `.term-btn` class below (not Tailwind utilities) carries that colour, since it
  // must hold regardless of inherited `color`. `color-mix` needs WebView2 (Chromium 111+)
  // or WebKitGTK 2.40+; older engines simply keep the plain scheme background/foreground.
  const toolBtn =
    'term-btn grid h-6 w-6 place-items-center rounded transition ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
  const toolBtnStyle = 'background: color-mix(in srgb, var(--term-fg) 8%, var(--term-bg));';
  const titleBtn =
    'term-btn grid h-5 w-5 place-items-center rounded transition ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';

  /** The action's current shortcut, formatted for a title, or empty when unbound. */
  function hint(action: TerminalAction): string {
    const chord = $terminalShortcuts[action];
    return chord ? ` (${formatChord(chord, isMac)})` : '';
  }

  // Session log (plan N): the backend records this pane's raw output into a file.
  const logging = $derived(termId != null && $sessionLogs.has(termId));
  const canLog = $derived(termId != null && status === 'connected');
  /** The save dialog is open: a second click must not open another. */
  let picking = $state(false);
  /** The stop button's tooltip: the file and how much is in it, read on hover. */
  let logTitle = $state('');
  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function toggleLog(): Promise<void> {
    const id = termId;
    if (id == null || picking) return;
    if (logging) {
      await logStop(id).catch((e) => lastError.set(`Stop logging: ${message(e)}`));
      return;
    }
    picking = true;
    try {
      await startSessionLog({
        sessionId: id,
        name: hostName,
        detail: sshLogDetail(get(hosts).find((h) => h.name === hostName)),
        mode: 'text',
        timestamps: get(logTimestamps)
      });
    } catch (e) {
      lastError.set(`Logging failed: ${message(e)}`);
    } finally {
      picking = false;
    }
  }

  async function refreshLogTitle(): Promise<void> {
    if (termId == null || !logging) return;
    const s = await logStatus(termId).catch(() => null);
    logTitle = s ? `Stop logging (${s.path}, ${formatBytes(s.bytes)})` : '';
  }

  function exportContents(): void {
    if (!term || picking) return;
    const text = bufferText(term.buffer.active);
    picking = true;
    exportText(hostName, text)
      .catch((e) => lastError.set(`Export failed: ${message(e)}`))
      .finally(() => (picking = false));
  }
</script>

<!-- The log and export buttons, in the hover toolbar (one pane) or the title bar
     (split). Recording shows a filled red dot. -->
{#snippet logButtons(cls: string, style: string, size: number)}
  <button
    type="button"
    class="{cls} disabled:cursor-not-allowed disabled:opacity-40"
    {style}
    title={logging ? logTitle || 'Stop logging' : 'Start logging'}
    aria-label={logging ? 'Stop logging' : 'Start logging'}
    aria-pressed={logging}
    disabled={!canLog || picking}
    onpointerenter={refreshLogTitle}
    onfocus={refreshLogTitle}
    onclick={toggleLog}
  >
    {#if logging}
      <span class="grid place-items-center text-status-crit"><Icon name="recording" {size} /></span>
    {:else}
      <Icon name="record" {size} />
    {/if}
  </button>
  <button
    type="button"
    class="{cls} disabled:cursor-not-allowed disabled:opacity-40"
    {style}
    title="Export contents"
    aria-label="Export contents"
    disabled={!ready || picking}
    onclick={exportContents}
  >
    <Icon name="download" {size} />
  </button>
{/snippet}

<!-- Clicking anywhere in the pane, or xterm's textarea taking focus, makes it the
     tab's focused pane. -->
<div
  class="group relative h-full w-full {framed ? 'p-0.5' : ''}"
  role="presentation"
  onpointerdown={onFocus}
  onfocusin={focusIn}
>
  <div
    class="flex h-full w-full flex-col overflow-hidden rounded"
    style={framed && focused
      ? 'box-shadow: 0 0 0 1px color-mix(in srgb, var(--term-fg) 50%, transparent);'
      : ''}
  >
    {#if titled}
      <!-- Drag the free area to move the pane; the buttons act as buttons. A mousedown
           on the free area is prevented from moving DOM focus onto this div itself
           (only a real focus target, like a button, should ever hold focus here) — see
           the focus $effect above, which otherwise couldn't tell the pane's own
           keyboard focus from a stray one parked on the title bar. -->
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -- pointer/mouse handlers only start a drag or keep focus off the free area; the buttons inside are the actual interactive controls. -->
      <div
        bind:this={titleBar}
        data-titlebar
        class="flex h-6 shrink-0 cursor-grab items-center gap-1.5 border-b px-1.5 text-xs active:cursor-grabbing"
        style="background: color-mix(in srgb, var(--term-fg) 8%, var(--term-bg)); border-color: color-mix(in srgb, var(--term-fg) 15%, var(--term-bg));"
        role="group"
        aria-label="Pane {hostName}"
        onpointerdown={(e) => {
          if (e.button === 0 && !(e.target as Element).closest('button')) onMoveStart(e);
        }}
        onmousedown={(e) => {
          if (!(e.target as Element).closest('button')) e.preventDefault();
        }}
      >
        <StatusDot status={sessionStatusDot[status]} size={7} />
        <span
          class="min-w-0 flex-1 truncate"
          style="color: var(--term-fg);"
          title={title || hostName}
        >
          {title || hostName}
        </span>
        {@render logButtons(titleBtn, '', 13)}
        <button type="button" class={titleBtn} title="Split right{hint('splitRight')}" aria-label="Split right" onclick={() => onSplit('row')}>
          <Icon name="splitRight" size={13} />
        </button>
        <button type="button" class={titleBtn} title="Split down{hint('splitDown')}" aria-label="Split down" onclick={() => onSplit('column')}>
          <Icon name="splitDown" size={13} />
        </button>
        <button type="button" class={titleBtn} title="Close pane{hint('closePane')}" aria-label="Close pane" onclick={onClose}>
          <Icon name="close" size={13} />
        </button>
      </div>
    {/if}
    <div bind:this={container} class="min-h-0 w-full flex-1" class:term-fade={scrolled}></div>
  </div>
  {#if !titled}
    <div
      bind:this={toolbar}
      class="peer absolute right-1.5 top-1.5 z-10 flex gap-1 opacity-0 transition group-hover:opacity-100 focus-within:opacity-100"
    >
      {@render logButtons(toolBtn, toolBtnStyle, 14)}
      <button type="button" class={toolBtn} style={toolBtnStyle} title="Split right{hint('splitRight')}" aria-label="Split right" onclick={() => onSplit('row')}>
        <Icon name="splitRight" size={14} />
      </button>
      <button type="button" class={toolBtn} style={toolBtnStyle} title="Split down{hint('splitDown')}" aria-label="Split down" onclick={() => onSplit('column')}>
        <Icon name="splitDown" size={14} />
      </button>
      <button type="button" class={toolBtn} style={toolBtnStyle} title="Close pane{hint('closePane')}" aria-label="Close pane" onclick={onClose}>
        <Icon name="close" size={14} />
      </button>
    </div>
    {#if logging}
      <!-- The toolbar shows only on hover or focus; recording shows all the time, and
           steps aside while the toolbar is up. -->
      <div
        class="pointer-events-none absolute right-2.5 top-2.5 z-10 text-status-crit transition group-hover:opacity-0 peer-focus-within:opacity-0"
        role="img"
        aria-label="Recording"
      >
        <Icon name="recording" size={12} />
      </div>
    {/if}
  {/if}
  {#if searchOpen && searchAddon}
    <div
      bind:this={searchBox}
      class="absolute right-1.5 {titled ? 'top-8' : 'top-9'} z-20 max-w-[calc(100%-0.75rem)]"
    >
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

<style>
  /* Scrolled output dissolves into the top edge instead of hard-clipping (on only while
     scrolled, so the first line stays crisp). black/transparent are mask alphas. */
  .term-fade {
    -webkit-mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
    mask-image: linear-gradient(to bottom, transparent, black 2.25rem);
  }

  /* The title-bar and hover-toolbar buttons' colour, keyed to --term-fg/--term-bg rather
     than the app's `text-muted`/`hover:bg-surface` tokens, so they stay legible when a
     dark-only scheme (Nord, Dracula) is shown under a light app theme. focus-visible
     keeps the app's own ring (Tailwind classes on the buttons themselves). */
  .term-btn {
    color: color-mix(in srgb, var(--term-fg) 70%, var(--term-bg));
  }
  .term-btn:hover {
    background: color-mix(in srgb, var(--term-fg) 15%, var(--term-bg));
    color: var(--term-fg);
  }
</style>
