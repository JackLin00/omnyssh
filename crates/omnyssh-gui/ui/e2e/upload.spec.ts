import { expect, test, type Locator, type Page } from '@playwright/test';

// Drag-and-drop upload (plan Q). Tauri is absent, so the stub stands in for it: OS file
// drags arrive as the `tauri://drag-*` events Tauri emits (fired with `__drag`), the
// upload commands are recorded in `__calls`, and the backend's `upload-progress` /
// `upload-finished` are fired with `__fire`. `__refuse` makes the next upload command
// fail with that message.
const HOSTS = [
  { name: 'web-1', hostname: '10.0.0.5', user: 'deploy', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false },
  { name: 'nas', hostname: '10.0.0.9', user: 'me', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false, uploadProtocol: 'sftp', uploadDir: '~/drop' }
];
const DEVICES = [
  {
    name: 'board',
    config: { port: 'COM3', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' },
    enter: 'cr',
    notes: null,
    uploadProtocol: 'ymodem'
  }
];

type Call = { cmd: string; args: Record<string, unknown> };

async function boot(page: Page): Promise<void> {
  await page.addInitScript(
    ({ hosts, devices }) => {
      let cbid = 0;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      const chIndex: Record<number, number> = {};
      let nextSession = 0;
      const calls: Call[] = [];
      win.__calls = calls;
      win.__refuse = null;

      function send(chId: number, text: string): void {
        const cb = win[`__cb${chId}`] as ((m: unknown) => void) | undefined;
        const index = chIndex[chId] ?? 0;
        chIndex[chId] = index + 1;
        cb?.({ message: new TextEncoder().encode(text).buffer, index });
      }
      function fire(event: string, payload: unknown): void {
        for (const id of listeners[event] ?? []) {
          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
          cb?.({ event, id, payload });
        }
      }
      win.__fire = fire;
      // Physical pixels, as Tauri reports them.
      win.__drag = (type: string, x: number, y: number, paths: string[]) => {
        const position = { x: x * devicePixelRatio, y: y * devicePixelRatio };
        if (type === 'leave') fire('tauri://drag-leave', null);
        else if (type === 'over') fire('tauri://drag-over', { position });
        else fire(`tauri://drag-${type}`, { paths, position });
      };

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
              return Promise.resolve(hosts);
            case 'list_serial_devices':
              return Promise.resolve(devices);
            case 'list_quick_commands':
              return Promise.resolve([]);
            case 'set_tray_behavior':
              return Promise.resolve({ available: true, minimize: true });
            case 'terminal_open': {
              const chId = (args.onOutput as { id: number }).id;
              const sid = ++nextSession;
              setTimeout(() => send(chId, 'omnyssh-ready> '), 0);
              return Promise.resolve(sid);
            }
            case 'serial_list_ports':
              return Promise.resolve([{ name: 'COM3', description: null }]);
            case 'serial_open':
              return Promise.resolve(++nextSession);
            case 'terminal_upload':
            case 'serial_upload': {
              calls.push({ cmd, args });
              const refuse = win.__refuse as string | null;
              win.__refuse = null;
              return refuse ? Promise.reject({ message: refuse }) : Promise.resolve(null);
            }
            case 'upload_cancel':
            case 'terminal_write':
            case 'serial_write':
            case 'save_host':
            case 'save_serial_device':
              calls.push({ cmd, args });
              return Promise.resolve(null);
            case 'plugin:event|listen': {
              const { event, handler } = args as { event: string; handler: number };
              (listeners[event] ||= []).push(handler);
              return Promise.resolve(handler);
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
    { hosts: HOSTS, devices: DEVICES }
  );
  await page.goto('/');
  await expect(page.getByText('2 hosts')).toBeVisible();
}

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) => ((window as unknown as { __calls: Call[] }).__calls ?? []).filter((c) => c.cmd === cmd).map((c) => c.args),
    cmd
  );

const fire = (page: Page, event: string, payload: unknown) =>
  page.evaluate(([event, payload]) => (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire(event as string, payload), [event, payload] as const);

/** Drags `paths` from outside onto the middle of `target` (or a point), optionally
 *  dropping them. */
async function dragOnto(
  page: Page,
  target: Locator | { x: number; y: number },
  paths: string[],
  drop = true
): Promise<void> {
  let x: number;
  let y: number;
  if ('boundingBox' in target) {
    const box = (await target.boundingBox())!;
    [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
  } else {
    ({ x, y } = target);
  }
  const drag = (type: string, px: number, py: number) =>
    page.evaluate(
      ([type, px, py, paths]) =>
        (window as unknown as { __drag: (t: string, x: number, y: number, p: string[]) => void }).__drag(
          type as string,
          px as number,
          py as number,
          paths as string[]
        ),
      [type, px, py, paths] as const
    );
  await drag('enter', x, y);
  await drag('over', x, y);
  if (drop) await drag('drop', x, y);
}

const progress = (sessionId: number, over: Record<string, unknown> = {}) => ({
  sessionId,
  waiting: false,
  fileIndex: 0,
  fileCount: 1,
  fileName: 'fw.bin',
  sent: 0,
  total: 1000,
  ...over
});

async function openTerminal(page: Page, host: string): Promise<Locator> {
  await page.getByTitle(`sh on ${host}`).click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
  return page.locator('.xterm').first();
}

test('a file dropped on a terminal goes up by ZMODEM, keys stay out of the transfer', async ({ page }) => {
  await boot(page);
  const term = await openTerminal(page, 'web-1');

  // Hovering shows where it would go.
  await dragOnto(page, term, ['/home/me/fw.bin'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with ZMODEM to web-1');
  await dragOnto(page, term, ['/home/me/fw.bin']);
  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
    { sessionId: 1, protocol: 'zmodem', paths: ['/home/me/fw.bin'] }
  ]);

  const bar = page.getByRole('status', { name: 'Upload' });
  await expect(bar).toContainText('Starting ZMODEM…');
  await fire(page, 'upload-progress', progress(1, { waiting: true, fileName: '', total: 0 }));
  await expect(bar).toContainText('Waiting for rz…');
  await fire(page, 'upload-progress', progress(1, { sent: 500 }));
  await expect(page.getByTestId('upload-text')).toHaveText('fw.bin · 50%');

  // Typing goes nowhere while the protocol owns the stream.
  await term.click();
  await page.keyboard.type('ls');
  await page.waitForTimeout(100);
  expect(await calls(page, 'terminal_write')).toEqual([]);

  await bar.getByRole('button', { name: 'Cancel' }).click();
  await expect.poll(() => calls(page, 'upload_cancel')).toEqual([{ sessionId: 1 }]);
  await fire(page, 'upload-finished', { sessionId: 1, ok: false, message: 'cancelled' });
  await expect(bar).toHaveCount(0);

  await term.click();
  await page.keyboard.type('x');
  await expect.poll(async () => (await calls(page, 'terminal_write')).length).toBeGreaterThan(0);
});

test('the host setting and the pane override pick the protocol; SFTP leaves the keyboard alone', async ({ page }) => {
  await boot(page);
  const term = await openTerminal(page, 'nas');
  await dragOnto(page, term, ['/a.txt', '/b.txt'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with SFTP to nas');
  await dragOnto(page, term, ['/a.txt', '/b.txt']);
  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
    { sessionId: 1, protocol: 'sftp', paths: ['/a.txt', '/b.txt'] }
  ]);
  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Connecting for SFTP…');
  await term.click();
  await page.keyboard.type('y');
  await expect.poll(async () => (await calls(page, 'terminal_write')).length).toBeGreaterThan(0);
  // The picker is locked while the upload runs.
  await expect(page.getByLabel('Upload protocol')).toBeDisabled();
  await fire(page, 'upload-finished', { sessionId: 1, ok: true, message: 'uploaded 2 files (2.0 KB)' });

  // This pane only: switch to ZMODEM and drop again.
  await page.getByLabel('Upload protocol').selectOption('zmodem');
  await dragOnto(page, term, ['/c.bin']);
  await expect.poll(async () => (await calls(page, 'terminal_upload')).at(-1)).toEqual({
    sessionId: 1,
    protocol: 'zmodem',
    paths: ['/c.bin']
  });
});

test('a refused drop says why and leaves no progress bar; a drop elsewhere does nothing', async ({ page }) => {
  await boot(page);
  const term = await openTerminal(page, 'web-1');
  await page.evaluate(() => ((window as unknown as { __refuse: string }).__refuse = 'folders cannot be uploaded: photos'));
  await dragOnto(page, term, ['/home/me/photos']);
  await expect(page.getByText('Upload: folders cannot be uploaded: photos')).toBeVisible();
  await expect(page.getByRole('status', { name: 'Upload' })).toHaveCount(0);

  // The sidebar, top left, is not a terminal.
  await dragOnto(page, { x: 20, y: 120 }, ['/a'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
  await dragOnto(page, { x: 20, y: 120 }, ['/a']);
  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
  expect(await calls(page, 'terminal_upload')).toHaveLength(1);
});

test('a saved device uploads with its protocol; XMODEM takes one file', async ({ page }) => {
  await boot(page);
  await page.getByTitle('term on board').click();
  const view = page.locator('.xterm').first();
  await expect(view).toBeVisible();
  await expect(page.getByLabel('Upload protocol')).toHaveValue('ymodem');

  await dragOnto(page, view, ['/u-boot.bin', '/env.bin'], false);
  await expect(page.getByTestId('drop-overlay')).toContainText('Drop to upload with YMODEM to board');
  await expect(page.getByTestId('drop-overlay')).toContainText('U-Boot takes only the first file');
  await dragOnto(page, view, ['/u-boot.bin']);
  await expect.poll(() => calls(page, 'serial_upload')).toEqual([
    { sessionId: 1, protocol: 'ymodem', paths: ['/u-boot.bin'] }
  ]);
  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Starting YMODEM…');
  await fire(page, 'upload-progress', progress(1, { waiting: true, fileName: '', total: 0 }));
  await expect(page.getByRole('status', { name: 'Upload' })).toContainText('Waiting for the receiver…');
  await fire(page, 'upload-finished', { sessionId: 1, ok: true, message: 'uploaded u-boot.bin (1 B)' });

  await page.getByLabel('Upload protocol').selectOption('xmodem');
  await dragOnto(page, view, ['/a.bin', '/b.bin'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('XMODEM sends one file at a time');
  await dragOnto(page, view, ['/a.bin', '/b.bin']);
  await expect(page.getByText('XMODEM sends one file at a time')).toBeVisible();
  expect(await calls(page, 'serial_upload')).toHaveLength(1);
});

test('a receive-only serial tab takes no drops', async ({ page }) => {
  await boot(page);
  await page.getByTitle('monitor on board').click();
  const view = page.locator('.xterm').first();
  await expect(view).toBeVisible();
  await expect(page.getByLabel('Upload protocol')).toHaveCount(0);
  await dragOnto(page, view, ['/a.bin']);
  await expect(page.getByTestId('drop-overlay')).toHaveCount(0);
  expect(await calls(page, 'serial_upload')).toEqual([]);
});

