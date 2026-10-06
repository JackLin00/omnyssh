<script lang="ts">
  // Bottom region (tech-gui.md §2): context + host summary, and where background
  // errors surface (§3.5). The summary counts total / online / alert / offline
  // (§4.1); colour lives only in the status dots, per the brandbook.
  import { lastError, statusNotice } from '$lib/stores/notifications';
  import { loggingLabel } from '$lib/stores/sessionLogs';
  import { hostSummary } from '$lib/stores/hostSummary';
  import { revealPath } from '$lib/ipc/commands';
  import { StatusDot } from '$lib/theme';

  function reveal(path: string): void {
    revealPath(path).catch((e) => lastError.set(e instanceof Error ? e.message : String(e)));
  }
</script>

<footer
  class="col-span-2 col-start-1 row-start-2 flex items-center justify-between gap-4 border-t border-default bg-surface px-5 py-2 text-xs text-muted"
>
  <div class="flex min-w-0 items-center gap-3">
    {#if $lastError}
      <span class="min-w-0 truncate text-status-crit">{$lastError}</span>
    {:else if $statusNotice}
      <span class="min-w-0 truncate" title={$statusNotice.message}>{$statusNotice.message}</span>
      {#if $statusNotice.path}
        {@const path = $statusNotice.path}
        <button
          type="button"
          class="shrink-0 rounded text-fg underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus"
          onclick={() => reveal(path)}
        >
          Show in folder
        </button>
      {/if}
    {:else}
      <span class="min-w-0 truncate">Ready</span>
    {/if}
    {#if $loggingLabel}
      <span class="flex shrink-0 items-center gap-1.5 text-fg">
        <span class="h-2 w-2 rounded-full bg-status-crit" aria-hidden="true"></span>{$loggingLabel}
      </span>
    {/if}
  </div>
  <div class="flex shrink-0 items-center gap-3">
    <span>{$hostSummary.total} {$hostSummary.total === 1 ? 'host' : 'hosts'}</span>
    <span class="flex items-center gap-1.5">
      <StatusDot status="ok" label="online" />{$hostSummary.online} online
    </span>
    <span class="flex items-center gap-1.5">
      <StatusDot status="warn" label="alert" />{$hostSummary.alert} alert
    </span>
    <span class="flex items-center gap-1.5">
      <StatusDot status="off" label="offline" />{$hostSummary.offline} offline
    </span>
  </div>
</footer>
