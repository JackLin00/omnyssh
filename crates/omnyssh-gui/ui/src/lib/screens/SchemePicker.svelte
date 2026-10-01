<script lang="ts">
  // The terminal colour scheme picker (Settings → Terminal colors): one preview card per
  // scheme, each rendered from the scheme's own data — never a colour literal here (the
  // no-hardcoded-hex test forbids it). A click applies the scheme to every open terminal
  // through the `terminalScheme` store.
  import type { ITheme } from '@xterm/xterm';
  import { theme } from '$lib/stores/theme';
  import { terminalScheme } from '$lib/stores/terminalScheme';
  import { resolveScheme, TERMINAL_SCHEMES, type TerminalScheme } from '$lib/theme/terminalSchemes';

  const ANSI: (keyof ITheme)[] = [
    'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
    'brightBlack', 'brightRed', 'brightGreen', 'brightYellow',
    'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite'
  ];

  /** The variant name under the current app theme, or "Dark only" when the scheme has
   *  no light variant and the app is currently light. */
  function variantLabel(s: TerminalScheme, appTheme: 'light' | 'dark'): string {
    if (!s.lightName && appTheme === 'light') return 'Dark only';
    return appTheme === 'light' && s.lightName ? s.lightName : s.darkName;
  }
</script>

<div class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(14rem, 1fr));">
  {#each TERMINAL_SCHEMES as s (s.id)}
    {@const colors = resolveScheme(s.id, $theme)}
    <button
      type="button"
      class="rounded-lg border border-default bg-surface p-3 text-left transition hover:border-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus {$terminalScheme ===
      s.id
        ? 'ring-2 ring-focus'
        : ''}"
      aria-pressed={$terminalScheme === s.id}
      onclick={() => terminalScheme.set(s.id)}
    >
      <div class="mb-2 flex items-baseline justify-between gap-2">
        <span class="truncate text-sm font-medium">{s.name}</span>
        <span class="shrink-0 text-xs text-muted">{variantLabel(s, $theme)}</span>
      </div>
      <div
        class="mb-2 space-y-0.5 rounded p-2 font-mono text-[11px] leading-tight"
        style="background: {colors.background}; color: {colors.foreground};"
      >
        <div>
          <span style="color: {colors.green}">user@host</span>:<span style="color: {colors.blue}"
            >~</span
          >$ ls
        </div>
        <div>
          <span style="color: {colors.blue}">src/</span>
          <span style="color: {colors.green}">build.sh</span> README.md
        </div>
        <div style="color: {colors.yellow}">warning: deprecated flag</div>
        <div style="color: {colors.red}">error: connection refused</div>
      </div>
      <div class="grid grid-cols-8 gap-0.5">
        {#each ANSI as key (key)}
          <div class="h-3 w-full rounded-sm" style="background: {colors[key] as string};"></div>
        {/each}
      </div>
    </button>
  {/each}
</div>
