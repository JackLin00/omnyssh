import { expect, test, type Page } from '@playwright/test';

// Key generators. e2e runs the static SPA with Tauri absent, so `__TAURI_INTERNALS__`
// is stubbed at the boundary. The stub keeps the generators and the hosts the way
// the backend would (a rename or delete follows through to the hosts), records every
// save and answer, and fingerprints a key by its length. `__fire` plays the core's
// `generated-key-required`; the generator then runs in a real Worker.
const CAM = { name: 'cam', hostname: '192.168.11.1', user: 'root', port: 22, tags: [], source: 'manual', hasKey: false, localForwards: [], tunnelAutostart: false, forwardAgent: false };
const WEB = { name: 'web', hostname: 'web.example.com', user: 'root', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false };

/** Returns key text naming everything it was given. */
const ECHO = `async function generateKey(deviceInfo, address) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(deviceInfo.mac));
  return {
    privateKey: '-----BEGIN OPENSSH PRIVATE KEY-----\\n' + JSON.stringify({ deviceInfo, address, bytes: digest.byteLength }) + '\\n-----END OPENSSH PRIVATE KEY-----\\n',
    fingerprint: 'SHA256:from-the-code'
  };
}`;

type Generators = { enabled: boolean; generators: Array<{ name: string; code: string; credential: string }> };
type Call = { cmd: string; args: Record<string, unknown> };

async function boot(page: Page, generators: Generators, hosts: unknown[] = [CAM, WEB]): Promise<void> {
  await page.addInitScript(
    ({ generators, hosts }) => {
      let cbid = 0;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      const calls: Call[] = [];
      win.__calls = calls;
      const state = { generators: structuredClone(generators), hosts: structuredClone(hosts) as Array<Record<string, unknown>> };
      win.__state = state;

      function fire(event: string, payload: unknown): void {
        for (const id of listeners[event] ?? []) {
          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
          cb?.({ event, id, payload });
        }
      }
      win.__fire = fire;

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
              return Promise.resolve(structuredClone(state.hosts));
            case 'list_serial_devices':
            case 'list_quick_commands':
              return Promise.resolve([]);
            // The settings page asks for these as it opens.
            case 'set_tray_behavior':
              return Promise.resolve({ available: true, minimize: true });
            case 'load_update_config':
              return Promise.resolve({ checkOnStartup: false, skipVersion: '' });
            case 'list_key_generators':
              return Promise.resolve(structuredClone(state.generators));
            case 'set_key_generators_enabled':
              calls.push({ cmd, args });
              state.generators.enabled = args.enabled as boolean;
              return Promise.resolve(null);
            case 'save_key_generator': {
              calls.push({ cmd, args });
              const { previousName, generator } = args as { previousName: string | null; generator: Generators['generators'][0] };
              const i = state.generators.generators.findIndex((g) => g.name === previousName);
              if (i >= 0) state.generators.generators[i] = generator;
              else state.generators.generators.push(generator);
              for (const h of state.hosts) if (previousName && h.keyGenerator === previousName) h.keyGenerator = generator.name;
              return Promise.resolve(null);
            }
            case 'delete_key_generator': {
              calls.push({ cmd, args });
              state.generators.generators = state.generators.generators.filter((g) => g.name !== args.name);
              for (const h of state.hosts) {
                if (h.keyGenerator === args.name) {
                  delete h.keyGenerator;
                  delete h.deviceMac;
                  delete h.deviceRevision;
                }
              }
              return Promise.resolve(null);
            }
            case 'key_fingerprint':
              calls.push({ cmd, args });
              return Promise.resolve(`SHA256:len-${(args.privateKey as string).length}`);
            case 'answer_generated_key':
            case 'save_host':
            case 'duplicate_host':
              calls.push({ cmd, args });
              return Promise.resolve(null);
            case 'reload_hosts':
              fire('hosts-loaded', structuredClone(state.hosts));
              return Promise.resolve(null);
            case 'plugin:event|listen': {
              const { event, handler } = args as { event: string; handler: number };
              (listeners[event] ||= []).push(handler);
              return Promise.resolve(cbid);
            }
            default:
              return Promise.resolve(null);
          }
        },
        transformCallback: (cb: unknown) => {
          const id = ++cbid;
          win[`__cb${id}`] = cb;
          return id;
        },
        unregisterCallback: (id: number) => {
          delete win[`__cb${id}`];
        }
      };
    },
    { generators, hosts }
  );
  await page.goto('/');
  await expect(page.getByText(`${hosts.length} host`)).toBeVisible();
}

const calls = (page: Page, cmd: string): Promise<Array<Record<string, unknown>>> =>
  page.evaluate(
    (cmd) => ((window as unknown as { __calls: Call[] }).__calls ?? []).filter((c) => c.cmd === cmd).map((c) => c.args),
    cmd
  );

const fire = (page: Page, event: string, payload: unknown) =>
  page.evaluate(
    ([event, payload]) => (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire(event as string, payload),
    [event, payload] as const
  );

const ON: Generators = {
  enabled: true,
  generators: [{ name: 'vendor-a', code: ECHO, credential: '{"master_key":"00ff"}' }]
};

test('a login asking for a generated key gets it from the code, run in a worker', async ({ page }) => {
  await boot(page, ON);
  await fire(page, 'generated-key-required', {
    requestId: 3,
    hostName: 'cam',
    code: ECHO,
    credential: '{"master_key":"00ff"}',
    mac: '30:ae:7b:eb:17:82',
    revision: null,
    address: '192.168.11.1'
  });
  await expect.poll(() => calls(page, 'answer_generated_key')).toHaveLength(1);
  const [answer] = await calls(page, 'answer_generated_key');
  expect(answer.requestId).toBe(3);
  expect(answer.error).toBeNull();
  const echoed = JSON.parse((answer.privateKey as string).split('\n')[1]);
  // No revision: the key is left out, for the code's own default.
  expect(echoed).toEqual({
    deviceInfo: { credential: { master_key: '00ff' }, mac: '30:ae:7b:eb:17:82' },
    address: '192.168.11.1',
    bytes: 32
  });

  // Code that fails answers with why.
  await fire(page, 'generated-key-required', {
    requestId: 4,
    hostName: 'cam',
    code: 'async function generateKey() { throw new Error("master_key is undefined"); }',
    credential: '',
    mac: 'x',
    revision: 1,
    address: '192.168.11.1'
  });
  await expect.poll(() => calls(page, 'answer_generated_key')).toHaveLength(2);
  expect((await calls(page, 'answer_generated_key'))[1]).toEqual({
    requestId: 4,
    privateKey: null,
    error: 'master_key is undefined'
  });
});

async function openSettings(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Key generators' })).toBeVisible();
}

test('the switch is off at first and shows nothing else; on, generators can be added', async ({ page }) => {
  await boot(page, { enabled: false, generators: [] });
  await openSettings(page);
  const toggle = page.getByRole('switch', { name: 'Use key generators' });
  await expect(toggle).toHaveAttribute('aria-checked', 'false');
  await expect(page.getByRole('button', { name: 'Add key generator' })).toHaveCount(0);

  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-checked', 'true');
  expect(await calls(page, 'set_key_generators_enabled')).toEqual([{ enabled: true }]);

  await page.getByRole('button', { name: 'Add key generator' }).click();
  const editor = page.getByRole('dialog', { name: 'Add key generator' });
  await editor.getByLabel('Name').fill('vendor-a');
  await editor.getByLabel('Code').fill(ECHO);
  // A credential that is not JSON is not saved.
  await editor.getByLabel('Credential (JSON)').fill('{master_key: 1}');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect(editor.getByRole('alert')).toHaveText('The credential is not valid JSON');
  await editor.getByLabel('Credential (JSON)').fill('{"master_key":"00ff"}');
  await editor.getByRole('button', { name: 'Save' }).click();

  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await calls(page, 'save_key_generator')).toEqual([
    { previousName: null, generator: { name: 'vendor-a', code: ECHO, credential: '{"master_key":"00ff"}' } }
  ]);
  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('vendor-a');
});

test('the test panel runs the code and shows the key fingerprint, or why there is none', async ({ page }) => {
  await boot(page, ON);
  await openSettings(page);
  await page.getByRole('button', { name: 'Edit vendor-a' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit key generator' });
  await editor.getByLabel('Test MAC').fill('30:ae:7b:eb:17:82');
  await editor.getByLabel('Test address').fill('192.168.11.1');
  await editor.getByRole('button', { name: 'Run' }).click();
  await expect(editor.getByTestId('test-fingerprint')).toHaveText(/^SHA256:len-\d+$/);
  // The stub's fingerprint is not the one the code reports.
  await expect(editor.getByText("The fingerprint the code returned is not this key's.")).toBeVisible();
  const [fingerprinted] = await calls(page, 'key_fingerprint');
  expect(fingerprinted.privateKey as string).toContain('"mac":"30:ae:7b:eb:17:82"');

  await editor.getByLabel('Code').fill('async function generateKey() { return { publicKey: "x" }; }');
  await editor.getByRole('button', { name: 'Run' }).click();
  await expect(editor.getByTestId('test-error')).toHaveText('The result has no privateKey');
});

test('a rename and a delete reach the hosts that use the generator', async ({ page }) => {
  await boot(page, ON, [{ ...CAM, keyGenerator: 'vendor-a', deviceMac: '30:ae:7b:eb:17:82' }, WEB]);
  await openSettings(page);
  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('1 host');

  await page.getByRole('button', { name: 'Edit vendor-a' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit key generator' });
  await editor.getByLabel('Name').fill('vendor-b');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect((await calls(page, 'save_key_generator'))[0].previousName).toBe('vendor-a');
  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('vendor-b');
  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('1 host');

  await page.getByRole('button', { name: 'Delete vendor-b' }).click();
  const confirm = page.getByRole('dialog', { name: 'Delete key generator' });
  await expect(confirm).toContainText('cam uses it');
  await confirm.getByRole('button', { name: 'Delete' }).click();
  expect(await calls(page, 'delete_key_generator')).toEqual([{ name: 'vendor-b' }]);
  await expect(page.getByRole('list', { name: 'Key generators' })).toContainText('No key generators yet.');
});

const SET_UP = { ...CAM, keyGenerator: 'vendor-a', deviceMac: '30:ae:7b:eb:17:82', deviceRevision: 2 };

test('while generators are off the host form and the cards show nothing of them', async ({ page }) => {
  await boot(page, { ...ON, enabled: false }, [SET_UP, WEB]);
  await expect(page.getByText('gen', { exact: true })).toHaveCount(0);
  await page.getByRole('button', { name: 'Edit cam' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit host' });
  await expect(editor.getByLabel('Key generator')).toHaveCount(0);
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect.poll(() => calls(page, 'save_host')).toHaveLength(1);
  // Left out, so the backend keeps the host's generator.
  expect('keyGen' in ((await calls(page, 'save_host'))[0].input as object)).toBe(false);
});

test('a host picks a generator with its MAC and revision; a duplicate keeps them', async ({ page }) => {
  await boot(page, ON, [SET_UP, WEB]);
  await expect(page.getByText('gen', { exact: true })).toHaveCount(1);
  await expect(page.getByText('gen', { exact: true })).toHaveAttribute('title', /vendor-a/);

  await page.getByRole('button', { name: 'Edit web' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit host' });
  await expect(editor.getByLabel('MAC')).toHaveCount(0);
  await editor.getByLabel('Key generator').selectOption('vendor-a');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect(editor.getByText('MAC is required for a key generator')).toBeVisible();
  await editor.getByLabel('MAC').fill('30:ae:7b:eb:17:99');
  await editor.getByLabel('Revision').fill('3');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect.poll(() => calls(page, 'save_host')).toHaveLength(1);
  expect(((await calls(page, 'save_host'))[0].input as { keyGen: unknown }).keyGen).toEqual({
    generator: 'vendor-a',
    mac: '30:ae:7b:eb:17:99',
    revision: 3
  });

  await page.getByRole('button', { name: 'Duplicate cam' }).click();
  const copy = page.getByRole('dialog', { name: 'Add host' });
  await expect(copy.getByLabel('MAC')).toHaveValue('30:ae:7b:eb:17:82');
  await copy.getByRole('button', { name: 'Add host' }).click();
  await expect.poll(() => calls(page, 'duplicate_host')).toHaveLength(1);
  expect(((await calls(page, 'duplicate_host'))[0].input as { keyGen: unknown }).keyGen).toEqual({
    generator: 'vendor-a',
    mac: '30:ae:7b:eb:17:82',
    revision: 2
  });
});
