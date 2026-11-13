<script lang="ts">
  // Add or edit a key generator: its name, its code and its credential, and a test
  // panel that runs the code as a login would and shows the key's fingerprint, read
  // by the backend as a login reads it. The private key itself is never shown.
  import type { KeyGeneratorDto } from '$lib/bindings';
  import { Button } from '$lib/theme';
  import Modal from '$lib/components/Modal.svelte';
  import { keyFingerprint } from '$lib/ipc/commands';
  import { runGenerator } from '$lib/keyGen';
  import { fingerprintsAgree, generatorFromFields, revisionError, type GeneratorFields } from './keyGenForm';

  let {
    mode,
    initial,
    taken,
    onSubmit,
    onCancel
  }: {
    mode: 'add' | 'edit';
    initial: GeneratorFields;
    /** The other generators' names. */
    taken: readonly string[];
    onSubmit: (generator: KeyGeneratorDto) => Promise<void>;
    onCancel: () => void;
  } = $props();

  // Seeded once; the editor is remounted per open.
  // svelte-ignore state_referenced_locally
  let fields = $state<GeneratorFields>({ ...initial });
  let error = $state<string | null>(null);
  let saving = $state(false);

  let testMac = $state('');
  let testRevision = $state('');
  let testAddress = $state('');
  type Outcome = { kind: 'ok'; fingerprint: string; agrees: boolean } | { kind: 'error'; message: string };
  let outcome = $state<Outcome | null>(null);
  let running = $state(false);

  const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

  async function runTest(): Promise<void> {
    const revision = testRevision.trim();
    const bad = revisionError(revision);
    if (bad) {
      outcome = { kind: 'error', message: bad };
      return;
    }
    running = true;
    outcome = null;
    try {
      const key = await runGenerator({
        code: fields.code,
        credential: fields.credential,
        mac: testMac.trim(),
        revision: revision === '' ? null : Number(revision),
        address: testAddress.trim()
      });
      const fingerprint = await keyFingerprint(key.privateKey);
      outcome = { kind: 'ok', fingerprint, agrees: fingerprintsAgree(fingerprint, key.fingerprint) };
    } catch (e) {
      outcome = { kind: 'error', message: message(e) };
    } finally {
      running = false;
    }
  }

  async function save(): Promise<void> {
    const result = generatorFromFields(fields, taken);
    if (!result.ok) {
      error = result.error;
      return;
    }
    error = null;
    saving = true;
    try {
      await onSubmit(result.generator);
    } catch (e) {
      error = message(e);
    } finally {
      saving = false;
    }
  }

  const label = 'block space-y-1 text-xs font-medium text-muted';
  const field =
    'w-full rounded-lg bg-surface-inset px-3 py-2 text-sm text-fg outline-none ' +
    'focus-visible:ring-2 focus-visible:ring-focus placeholder:text-faint';
  const mono = `${field} resize-y font-mono text-xs`;
</script>

<Modal label={mode === 'add' ? 'Add key generator' : 'Edit key generator'} onClose={onCancel}>
  <form
    class="flex max-h-[76vh] w-[40rem] max-w-full flex-col"
    onsubmit={(e) => {
      e.preventDefault();
      void save();
    }}
  >
    <header class="border-b border-default px-5 py-3.5">
      <h2 class="text-sm font-semibold">{mode === 'add' ? 'Add key generator' : 'Edit key generator'}</h2>
    </header>
    <div class="min-h-0 flex-1 space-y-3.5 overflow-y-auto px-5 py-4">
      <label class={label}>
        <span>Name</span>
        <!-- svelte-ignore a11y_autofocus -->
        <input bind:value={fields.name} class={field} autocomplete="off" autofocus />
      </label>
      <label class={label}>
        <span>Code</span>
        <textarea bind:value={fields.code} rows="12" class={mono} spellcheck="false"></textarea>
      </label>
      <p class="text-xs text-faint">
        Defines <span class="font-mono">generateKey(deviceInfo, address)</span>, called with
        <span class="font-mono">{'{ credential, mac, revision }'}</span> and the host's address, returning
        <span class="font-mono">{'{ privateKey, … }'}</span>. Runs in a worker of its own: Web Crypto and plain JS.
      </p>
      <label class={label}>
        <span>Credential (JSON)</span>
        <textarea bind:value={fields.credential} rows="4" class={mono} spellcheck="false" placeholder={'{ "master_key": "…" }'}
        ></textarea>
      </label>

      <fieldset class="space-y-2 rounded-lg border border-default p-3">
        <legend class="px-1 text-xs font-medium text-muted">Test</legend>
        <div class="grid grid-cols-[1fr,6rem,1fr] gap-2">
          <input bind:value={testMac} class={field} placeholder="MAC" aria-label="Test MAC" autocomplete="off" />
          <input bind:value={testRevision} class={field} placeholder="Revision" aria-label="Test revision" inputmode="numeric" />
          <input bind:value={testAddress} class={field} placeholder="Address" aria-label="Test address" autocomplete="off" />
        </div>
        <!-- Announced: the result arrives a moment after Run. -->
        <div class="flex items-center gap-3" aria-live="polite">
          <Button variant="ghost" onclick={runTest} disabled={running}>{running ? 'Running…' : 'Run'}</Button>
          {#if outcome?.kind === 'ok'}
            <span class="min-w-0 truncate font-mono text-xs text-fg" data-testid="test-fingerprint">{outcome.fingerprint}</span>
          {:else if outcome?.kind === 'error'}
            <span class="min-w-0 text-xs text-status-crit" data-testid="test-error">{outcome.message}</span>
          {/if}
        </div>
        {#if outcome?.kind === 'ok' && !outcome.agrees}
          <p class="text-xs text-status-warn" role="status">The fingerprint the code returned is not this key's.</p>
        {/if}
      </fieldset>

      {#if error}<p class="text-sm text-status-crit" role="alert">{error}</p>{/if}
    </div>
    <footer class="flex justify-end gap-2 border-t border-default px-5 py-3">
      <Button variant="ghost" onclick={onCancel}>Cancel</Button>
      <Button variant="primary" type="submit" disabled={saving}>Save</Button>
    </footer>
  </form>
</Modal>
