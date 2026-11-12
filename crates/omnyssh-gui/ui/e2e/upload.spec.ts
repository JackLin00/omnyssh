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

async function boot(page: Page, holdConnect = false): Promise<void> {
  await page.addInitScript(
    ({ hosts, devices, holdConnect }) => {
      let cbid = 0;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      const chIndex: Record<number, number> = {};
      let nextSession = 0;
      let exitChannel = 0;
      win.__exitSerial = (error: string) =>
        (win[`__cb${exitChannel}`] as ((m: unknown) => void) | undefined)?.({ message: { error }, index: 0 });
      const calls: Call[] = [];
      win.__calls = calls;
      win.__refuse = null;
      // With `holdConnect` the terminal opens but prints nothing (the pane stays
      // "connecting") until `__releaseConnect()`; an early release is remembered.
      const held: number[] = [];
      let released = false;
      win.__releaseConnect = () => {
        released = true;
        for (const chId of held.splice(0)) send(chId, 'omnyssh-ready> ');
      };

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
      // Page pixels x devicePixelRatio, which is what WebView2 (Windows) reports and the
      // listener divides back out. Playwright's Desktop Chrome profile has a Windows user
      // agent and a ratio of 1.
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
              // Shell output on the latest terminal, for the test to send.
              win.__output = (text: string) => send(chId, text);
              if (holdConnect && !released) held.push(chId);
              else setTimeout(() => send(chId, 'omnyssh-ready> '), 0);
              return Promise.resolve(sid);
            }
            case 'serial_list_ports':
              return Promise.resolve([{ name: 'COM3', description: null }]);
            case 'serial_open':
              exitChannel = (args as { onExit: { id: number } }).onExit.id;
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
    { hosts: HOSTS, devices: DEVICES, holdConnect }
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
    { sessionId: 1, protocol: 'zmodem', paths: ['/home/me/fw.bin'], dir: null }
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

  // Cancel must not leave the keyboard stranded: no click on the terminal first.
  await page.keyboard.type('x');
  await expect.poll(async () => (await calls(page, 'terminal_write')).length).toBeGreaterThan(0);
});

test('the host setting and the pane override pick the protocol; SFTP leaves the keyboard alone', async ({ page }) => {
  await boot(page);
  const term = await openTerminal(page, 'nas');
  await dragOnto(page, term, ['/a.txt', '/b.txt'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with SFTP to nas:~/drop');
  await dragOnto(page, term, ['/a.txt', '/b.txt']);
  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
    { sessionId: 1, protocol: 'sftp', paths: ['/a.txt', '/b.txt'], dir: null }
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
    paths: ['/c.bin'],
    dir: null
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

test('the host editor sets SFTP and its folder', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Edit web-1' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit host' });
  await expect(editor.getByLabel('Upload folder')).toHaveCount(0);
  await editor.getByLabel('Drag-and-drop upload').selectOption('sftp');
  await editor.getByLabel('Upload folder').fill('/srv/in');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect.poll(async () => (await calls(page, 'save_host'))[0]).toMatchObject({
    input: { name: 'web-1', uploadProtocol: 'sftp', uploadDir: '/srv/in' }
  });
});

test('the serial device editor keeps the upload protocol', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Edit board' }).click();
  const editor = page.getByRole('dialog', { name: 'Edit serial device' });
  await expect(editor.getByLabel('Drag-and-drop upload')).toHaveValue('ymodem');
  await editor.getByLabel('Drag-and-drop upload').selectOption('xmodem');
  await editor.getByRole('button', { name: 'Save' }).click();
  await expect.poll(async () => (await calls(page, 'save_serial_device'))[0]).toMatchObject({
    device: { name: 'board', uploadProtocol: 'xmodem' }
  });
});

test('a serial upload ends when its port exits', async ({ page }) => {
  await boot(page);
  await page.getByTitle('term on board').click();
  const view = page.locator('.xterm').first();
  await expect(view).toBeVisible();
  await dragOnto(page, view, ['/u-boot.bin']);
  await expect(page.getByRole('status', { name: 'Upload' })).toBeVisible();
  await expect(page.getByLabel('Upload protocol')).toBeDisabled();

  await page.evaluate(() => (window as unknown as { __exitSerial: (e: string) => void }).__exitSerial('device unplugged'));
  await expect(page.getByRole('status', { name: 'Upload' })).toHaveCount(0);
  await expect(page.getByLabel('Upload protocol')).toBeEnabled();
  await dragOnto(page, view, ['/u-boot.bin'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('COM3 is not open');
});

test('keystrokes are dropped during a serial upload', async ({ page }) => {
  await boot(page);
  await page.getByTitle('term on board').click();
  const view = page.locator('.xterm').first();
  await expect(view).toBeVisible();
  await view.click();
  await page.keyboard.type('a');
  await expect.poll(() => calls(page, 'serial_write')).toHaveLength(1);

  await dragOnto(page, view, ['/u-boot.bin']);
  await expect(page.getByRole('status', { name: 'Upload' })).toBeVisible();
  await view.click();
  await page.keyboard.type('b');
  await page.waitForTimeout(300);
  expect(await calls(page, 'serial_write')).toHaveLength(1);
});

test('cancelling a serial upload gives the keyboard back', async ({ page }) => {
  await boot(page);
  await page.getByTitle('term on board').click();
  const view = page.locator('.xterm').first();
  await expect(view).toBeVisible();
  await view.click();
  await dragOnto(page, view, ['/u-boot.bin']);
  const bar = page.getByRole('status', { name: 'Upload' });
  await bar.getByRole('button', { name: 'Cancel' }).click();
  await expect.poll(() => calls(page, 'upload_cancel')).toEqual([{ sessionId: 1 }]);
  await fire(page, 'upload-finished', { sessionId: 1, ok: false, message: 'cancelled' });
  await expect(bar).toHaveCount(0);
  await page.keyboard.type('x');
  await expect.poll(async () => (await calls(page, 'serial_write')).length).toBeGreaterThan(0);
});

test('a terminal still connecting takes no drops until it is connected', async ({ page }) => {
  await boot(page, true);
  await page.getByTitle('sh on web-1').click();
  const term = page.locator('.xterm').first();
  await expect(term).toBeVisible();

  await dragOnto(page, term, ['/a.bin'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('The terminal is still connecting');
  await dragOnto(page, term, ['/a.bin']);
  expect(await calls(page, 'terminal_upload')).toEqual([]);

  await page.evaluate(() => (window as unknown as { __releaseConnect: () => void }).__releaseConnect());
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
  await dragOnto(page, term, ['/a.bin']);
  await expect.poll(() => calls(page, 'terminal_upload')).toEqual([
    { sessionId: 1, protocol: 'zmodem', paths: ['/a.bin'], dir: null }
  ]);
});

test('SFTP drops follow the folder the shell reports, once it is set up to', async ({ page }) => {
  await boot(page);
  const term = await openTerminal(page, 'nas');
  const output = (text: string) =>
    page.evaluate((t) => (window as unknown as { __output: (t: string) => void }).__output(t), text);

  // Until the shell reports a folder, the host's upload folder, and an offer to set it up.
  const follow = page.getByRole('button', { name: "Follow the shell's folder" });
  await follow.click();
  await expect
    .poll(async () =>
      (await calls(page, 'terminal_write')).map((a) => new TextDecoder().decode(new Uint8Array(a.data as number[]))).join('')
    )
    .toContain('__omny_cwd');

  // The shell now says where it is (OSC 7), after each command.
  await output('\x1b]7;file://nas-box/var/log\x1b\\$ ');
  await expect(follow).toHaveCount(0);
  await dragOnto(page, term, ['/a.txt'], false);
  await expect(page.getByTestId('drop-overlay')).toHaveText('Drop to upload with SFTP to nas:/var/log');
  await dragOnto(page, term, ['/a.txt']);
  await expect.poll(async () => (await calls(page, 'terminal_upload')).at(-1)).toEqual({
    sessionId: 1,
    protocol: 'sftp',
    paths: ['/a.txt'],
    dir: '/var/log'
  });
  await fire(page, 'upload-finished', { sessionId: 1, ok: true, message: 'uploaded 1 file' });

  // `ssh` on to another machine: its folders are not where SFTP connects.
  await output('\x1b]7;file://other/tmp\x1b\\$ ');
  await dragOnto(page, term, ['/b.txt']);
  await expect.poll(async () => (await calls(page, 'terminal_upload')).at(-1)?.dir).toBeNull();
});
