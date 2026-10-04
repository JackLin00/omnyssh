import { expect, test, type Page } from '@playwright/test';

// Changed host keys (tech-gui.md §4.2/§4.3). e2e runs against the static SPA with
// Tauri absent, so `__TAURI_INTERNALS__` is stubbed at the boundary (§6.4). The
// stub plays the core: opening files on a host whose key changed sends
// `host-key-changed`, `answer_host_key` records the choice, and the connect
// resolves on update/once or fails on cancel, as the core's does.
const HOSTS = [
  { name: 'nas', hostname: 'nas.example.com', user: 'admin', port: 22, tags: [], source: 'sshConfig', hasKey: false, localForwards: [], tunnelAutostart: false, forwardAgent: false }
];

type Answer = { requestId: number; decision: string };

const QUESTION = {
  hostName: 'nas',
  host: 'nas.example.com',
  port: 22,
  jumpFor: null,
  file: '/home/me/.ssh/known_hosts',
  saved: [{ keyType: 'ssh-ed25519', fingerprint: 'SHA256:oldOLDold' }],
  offered: { keyType: 'ssh-ed25519', fingerprint: 'SHA256:newNEWnew' }
};

async function boot(page: Page): Promise<void> {
  await page.addInitScript(
    ({ hosts, question }) => {
      let cbid = 0;
      let request = 0;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      const answers: Array<{ requestId: number; decision: string }> = [];
      win.__answers = answers;

      function fire(event: string, payload: unknown): void {
        for (const id of listeners[event] ?? []) {
          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
          cb?.({ event, id, payload });
        }
      }
      win.__fire = fire;

      let waiting: ((ok: boolean) => void) | null = null;

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
              return Promise.resolve(hosts);
            case 'list_serial_devices':
            case 'list_quick_commands':
              return Promise.resolve([]);
            case 'sftp_open': {
              request += 1;
              const requestId = request;
              setTimeout(() => fire('host-key-changed', { ...question, requestId }), 0);
              return new Promise((resolve, reject) => {
                waiting = (ok) =>
                  ok ? resolve(11) : reject({ message: 'SFTP SSH connect: Host key of nas.example.com has changed' });
              });
            }
            case 'list_local_roots':
              return Promise.resolve(['/']);
            case 'answer_host_key': {
              const { requestId, decision } = args as { requestId: number; decision: string };
              // Only the question the stub asked last is waited on, as in the core.
              if (requestId !== request) return Promise.reject({ message: 'no connection is waiting for this answer' });
              answers.push({ requestId, decision });
              waiting?.(decision !== 'cancel');
              return Promise.resolve(null);
            }
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
    { hosts: HOSTS, question: QUESTION }
  );
  await page.goto('/');
  await expect(page.getByText('1 host')).toBeVisible();
}

const answers = (page: Page): Promise<Answer[]> =>
  page.evaluate(() => (window as unknown as { __answers: Answer[] }).__answers);

const fire = (page: Page, payload: unknown): Promise<void> =>
  page.evaluate((p) => {
    (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire('host-key-changed', p);
  }, payload);

test('a changed host key shows both keys, and Enter on the focused Cancel gives up', async ({ page }) => {
  await boot(page);
  await page.getByTitle('files on nas').click();

  const dialog = page.getByRole('dialog', { name: 'Host key changed' });
  await expect(dialog.getByText('Host key changed: nas.example.com')).toBeVisible();
  await expect(dialog.getByText('/home/me/.ssh/known_hosts')).toBeVisible();
  await expect(dialog.getByText('SHA256:oldOLDold')).toBeVisible();
  await expect(dialog.getByText('SHA256:newNEWnew')).toBeVisible();
  await expect(dialog.getByText('ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();

  await page.keyboard.press('Enter');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await answers(page)).toEqual([{ requestId: 1, decision: 'cancel' }]);
  await expect(page.getByRole('main').getByText('SFTP SSH connect: Host key of nas.example.com has changed')).toBeVisible();
});

test('Esc gives up too', async ({ page }) => {
  await boot(page);
  await page.getByTitle('files on nas').click();
  await expect(page.getByRole('dialog', { name: 'Host key changed' }).getByRole('button', { name: 'Cancel' })).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  expect(await answers(page)).toEqual([{ requestId: 1, decision: 'cancel' }]);
});

for (const [button, decision] of [
  ['Update key and connect', 'update'],
  ['Connect once', 'once']
] as const) {
  test(`${button} goes back as the choice alone`, async ({ page }) => {
    await boot(page);
    await page.getByTitle('files on nas').click();
    await page.getByRole('dialog', { name: 'Host key changed' }).getByRole('button', { name: button }).click();
    await expect(page.getByRole('dialog')).toHaveCount(0);
    expect(await answers(page)).toEqual([{ requestId: 1, decision }]);
    await expect(page.getByText('Host key of nas.example.com has changed')).toHaveCount(0);
  });
}

test('questions queue, and a jump host says whose it is', async ({ page }) => {
  await boot(page);
  await fire(page, { ...QUESTION, requestId: 40, host: 'bastion.example.com', port: 2222, jumpFor: 'nas' });
  await fire(page, { ...QUESTION, requestId: 41 });

  const dialog = page.getByRole('dialog', { name: 'Host key changed' });
  await expect(dialog.getByText('Host key changed: bastion.example.com:2222 (jump host for nas)')).toBeVisible();
  // The stub waits on neither, so the answer is refused and the dialog says so.
  await dialog.getByRole('button', { name: 'Connect once' }).click();
  await expect(page.getByText('That connection stopped waiting. Open it again.')).toBeVisible();
  await expect(dialog.getByText('Host key changed: nas.example.com')).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
});

test('a click beside the dialog decides nothing', async ({ page }) => {
  await boot(page);
  await page.getByTitle('files on nas').click();
  const dialog = page.getByRole('dialog', { name: 'Host key changed' });
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await page.mouse.click(5, 5);
  await expect(dialog).toBeVisible();
  expect(await answers(page)).toEqual([]);
});

test('streamer mode keeps the host out of the dialog title', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('omnyssh-streamer-mode', 'true'));
  await boot(page);
  await page.getByTitle('files on nas').click();
  const dialog = page.getByRole('dialog', { name: 'Host key changed' });
  await expect(dialog.getByRole('button', { name: 'Cancel' })).toBeFocused();
  await expect(dialog.getByRole('heading')).not.toContainText('nas.example.com');
});
