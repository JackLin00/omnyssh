<script lang="ts">
  // The find bar of one terminal: searches its xterm (screen and scrollback) through the
  // search addon, highlights every match, counts them, and steps between them. The
  // parent shows it on the find chord and bumps `focusToken` to focus and select the
  // query again; Esc or × closes it and hands the keyboard back to the terminal.
  import { onMount } from 'svelte';
  import { get } from 'svelte/store';
  import type { ISearchOptions, SearchAddon } from '@xterm/addon-search';
  import { Icon } from '$lib/theme';
  import { theme } from '$lib/stores/theme';
  import { searchDecorations } from '$lib/theme/terminalTheme';
  import { terminalShortcuts } from '$lib/stores/terminalShortcuts';
  import { matchTerminalAction } from './terminalShortcuts';
  import { formatResults, isValidRegex } from './terminalSearch';

  let {
    addon,
    focusToken,
    onClose
  }: { addon: SearchAddon; focusToken: number; onClose: () => void } = $props();

  let input = $state<HTMLInputElement>();
  let query = $state('');
  let caseSensitive = $state(false);
  let regex = $state(false);
  let results = $state<{ resultIndex: number; resultCount: number } | null>(null);
  const invalid = $derived(regex && query !== '' && !isValidRegex(query));

  onMount(() => {
    const off = addon.onDidChangeResults((r) => (results = r));
    return () => {
      off.dispose();
      addon.clearDecorations();
    };
  });

  function options(incremental: boolean): ISearchOptions {
    return { caseSensitive, regex, incremental, decorations: searchDecorations(get(theme)) };
  }

  function find(forward: boolean, incremental = false): void {
    if (query === '' || invalid) {
      addon.clearDecorations();
      results = null;
      return;
    }
    if (forward) addon.findNext(query, options(incremental));
    else addon.findPrevious(query, options(incremental));
  }

  // Search as the query or an option changes, keeping the current match where it is.
  $effect(() => {
    void query;
    void caseSensitive;
    void regex;
    find(true, true);
  });

  // Focus and select on open, and again whenever the parent asks.
  $effect(() => {
    void focusToken;
    requestAnimationFrame(() => {
      input?.focus();
      input?.select();
    });
  });
  onMount(() => input?.focus());

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      find(!e.shiftKey);
    } else if (matchTerminalAction(e, get(terminalShortcuts)) === 'find') {
      e.preventDefault();
      input?.select();
    }
  }

  const toggle = (on: boolean): string =>
    'grid h-6 min-w-6 place-items-center rounded px-1 font-mono text-[11px] transition ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus ' +
    (on ? 'bg-accent text-accent-fg' : 'text-muted hover:text-fg');
  const iconBtn =
    'grid h-6 w-6 place-items-center rounded text-muted transition hover:text-fg ' +
    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

<div
  class="flex items-center gap-1 rounded-lg border border-default bg-surface-raised p-1 shadow-soft"
  role="search"
>
  <input
    bind:this={input}
    bind:value={query}
    type="text"
    placeholder="Find"
    aria-label="Find in terminal"
    aria-invalid={invalid}
    class="w-44 rounded bg-surface-inset px-2 py-1 text-xs text-fg outline-none placeholder:text-faint focus-visible:ring-2 {invalid
      ? 'ring-2 ring-status-crit'
      : 'focus-visible:ring-focus'}"
    onkeydown={onKeydown}
  />
  <span class="min-w-[4.5rem] px-1 text-center text-[11px] tabular-nums text-muted" aria-live="polite">
    {invalid ? 'Bad regex' : results ? formatResults(results) : ''}
  </span>
  <button
    type="button"
    class={toggle(caseSensitive)}
    title="Match case"
    aria-label="Match case"
    aria-pressed={caseSensitive}
    onclick={() => (caseSensitive = !caseSensitive)}
  >
    Aa
  </button>
  <button
    type="button"
    class={toggle(regex)}
    title="Regular expression"
    aria-label="Regular expression"
    aria-pressed={regex}
    onclick={() => (regex = !regex)}
  >
    .*
  </button>
  <button type="button" class={iconBtn} title="Previous (Shift+Enter)" aria-label="Previous match" onclick={() => find(false)}>
    <Icon name="chevronUp" size={14} />
  </button>
  <button type="button" class={iconBtn} title="Next (Enter)" aria-label="Next match" onclick={() => find(true)}>
    <Icon name="chevronDown" size={14} />
  </button>
  <button type="button" class={iconBtn} title="Close (Esc)" aria-label="Close find" onclick={onClose}>
    <Icon name="close" size={14} />
  </button>
</div>
