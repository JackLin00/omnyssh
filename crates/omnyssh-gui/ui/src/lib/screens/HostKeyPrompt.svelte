<script lang="ts">
  // A server's host key no longer matches known_hosts (tech-gui.md §4.3). The
  // connection was turned down; this asks whether to save the new key and connect,
  // connect once, or give up. Only the choice goes back, never a key. Cancel holds
  // the focus, so Enter gives up, as does Esc.
  import Modal from '$lib/components/Modal.svelte';
  import { Button, Icon } from '$lib/theme';
  import type { HostKeyDecisionDto } from '$lib/bindings';
  import { hostKeyPrompt, hostKeyTitle, serverKeyFile, settleHostKey } from '$lib/stores/hostKey';
  import { streamerMode } from '$lib/stores/streamer';
  import { lastError } from '$lib/stores/notifications';
  import { answerHostKey } from '$lib/ipc/commands';

  let cancel = $state<HTMLButtonElement>();
  let box = $state<HTMLDivElement>();

  // Focus is taken outright for each question: a live terminal holding it would
  // otherwise turn a stray Enter into shell input instead of a Cancel.
  const requestId = $derived($hostKeyPrompt?.requestId);
  $effect(() => {
    void requestId;
    cancel?.focus();
  });

  // The dialog opens unbidden: focus goes back when the last question closes.
  const open = $derived($hostKeyPrompt !== null);
  let opener: Element | null = null;
  $effect.pre(() => {
    if (open) {
      opener = document.activeElement;
    } else {
      const here = document.activeElement === document.body || box?.contains(document.activeElement);
      if (here && opener instanceof HTMLElement) opener.focus();
      opener = null;
    }
  });

  async function answer(decision: HostKeyDecisionDto): Promise<void> {
    const id = requestId;
    if (id === undefined) return;
    // Settled first either way: the answer goes once. A connection that stopped
    // waiting (it gives up after a few minutes) refuses it; say so.
    settleHostKey(id);
    await answerHostKey(id, decision).catch(() =>
      lastError.set('That connection stopped waiting. Open it again.')
    );
  }

  const cancelClass =
    'inline-flex items-center justify-center gap-2 rounded-full bg-accent px-7 py-2.5 text-sm font-medium ' +
    'text-accent-fg transition hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-focus';
</script>

{#if $hostKeyPrompt}
  {@const q = $hostKeyPrompt}
  <!-- A stray click beside it must not decide anything. -->
  <Modal label="Host key changed" onClose={() => void answer('cancel')} backdropCloses={false}>
    <div bind:this={box} class="space-y-4 border-t-4 border-status-crit px-5 py-4">
      <div class="space-y-1">
        <div class="flex items-center gap-2.5 text-status-crit">
          <Icon name="shield" size={16} />
          <h2 class="min-w-0 break-all text-sm font-semibold">{hostKeyTitle(q, $streamerMode)}</h2>
        </div>
        <p class="text-xs text-muted">Opening {q.hostName}</p>
      </div>
      <div class="space-y-1 text-xs">
        <p class="font-medium text-muted">
          Saved in <span class="break-all font-mono">{q.file}</span>
        </p>
        {#each q.saved as key (key.fingerprint)}
          <p class="break-all font-mono text-fg">{key.keyType} {key.fingerprint}</p>
        {/each}
      </div>
      <div class="space-y-1 text-xs">
        <p class="font-medium text-muted">The server shows now</p>
        <p class="break-all font-mono text-status-crit">{q.offered.keyType} {q.offered.fingerprint}</p>
      </div>
      <p class="text-xs text-muted">
        If the server was reinstalled or its keys were replaced, this is expected. Otherwise someone may be
        intercepting the connection. Check the key on the server first, for example with
        <code class="break-all font-mono">ssh-keygen -lf {serverKeyFile(q.offered.keyType)}</code>.
      </p>
      <div class="flex flex-wrap justify-end gap-2">
        <Button variant="ghost" onclick={() => void answer('update')}>Update key and connect</Button>
        <Button variant="ghost" onclick={() => void answer('once')}>Connect once</Button>
        <button bind:this={cancel} type="button" class={cancelClass} onclick={() => void answer('cancel')}>
          Cancel
        </button>
      </div>
    </div>
  </Modal>
{/if}
