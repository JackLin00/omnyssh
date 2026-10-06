import { expect, test, type Page } from '@playwright/test';

// Session logs and exports (plan N). Tauri is absent, so the stub stands in for the
// backend: `pick_save_path` answers with `__nextPath` (null = the dialog was
// cancelled), the log and export commands are recorded in `__calls`, and
// `log_stop` answers with the `log-stopped` event the real backend emits.
const HOSTS = [
  { name: 'web-1', hostname: '10.0.0.5', user: 'deploy', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false }
];

type Call = { cmd: string; args: Record<string, unknown> };

async function boot(page: Page): Promise<void> {
  await page.addInitScript((hosts) => {
    let cbid = 0;
    const win = window as unknown as Record<string, unknown>;
    const listeners: Record<string, number[]> = {};
    const chIndex: Record<number, number> = {};
    let nextSession = 0;
    let serialChannel: number | undefined;
    const calls: Call[] = [];
    win.__calls = calls;
    win.__nextPath = '/logs/picked.log';

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
    win.__sendSerial = (text: string) => serialChannel != null && send(serialChannel, text);

    (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      invoke: (cmd: string, args: Record<string, unknown>) => {
        switch (cmd) {
          case 'list_hosts':
            return Promise.resolve(hosts);
          case 'list_serial_devices':
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
            serialChannel = (args.onOutput as { id: number }).id;
            return Promise.resolve(++nextSession);
          case 'pick_save_path':
            calls.push({ cmd, args });
            return Promise.resolve(win.__nextPath);
          case 'log_start':
          case 'save_text_file':
          case 'reveal_path':
            calls.push({ cmd, args });
            return Promise.resolve(null);
          case 'log_status':
            return Promise.resolve({ path: '/logs/picked.log', bytes: 2048 });
          case 'log_stop': {
            calls.push({ cmd, args });
            const sessionId = args.sessionId as number;
            setTimeout(() => fire('log-stopped', { sessionId, path: '/logs/picked.log', bytes: 2048, error: null }), 0);
            return Promise.resolve(null);
          }
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
  }, HOSTS);
  await page.goto('/');
  await expect(page.getByText('1 host')).toBeVisible();
}

const calls = (page: Page, cmd: string) =>
  page.evaluate(
    (cmd) => ((window as unknown as { __calls: Call[] }).__calls ?? []).filter((c) => c.cmd === cmd).map((c) => c.args),
    cmd
  );

const setNextPath = (page: Page, path: string | null) =>
  page.evaluate((path) => ((window as unknown as { __nextPath: string | null }).__nextPath = path), path);

async function openTerminal(page: Page): Promise<void> {
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
}

test('a pane logs into the picked file until it is stopped, then offers to show it', async ({ page }) => {
  await boot(page);
  await openTerminal(page);

  await page.getByRole('button', { name: 'Start logging' }).click();
  await expect
    .poll(() => calls(page, 'log_start'))
    .toEqual([
      {
        sessionId: 1,
        path: '/logs/picked.log',
        timestamps: true,
        mode: 'text',
        header: expect.stringMatching(/^# OmnySSH log: web-1 \(deploy@10\.0\.0\.5:22\), started \d{4}-\d\d-\d\d \d\d:\d\d:\d\d$/)
      }
    ]);
  const [pick] = await calls(page, 'pick_save_path');
  expect(pick).toEqual({ defaultName: expect.stringMatching(/^web-1_\d{4}-\d\d-\d\d_\d\d-\d\d-\d\d\.log$/), kind: 'log' });

  // Recording shows without hovering: the toolbar is faded out, the indicator is not.
  await page.mouse.move(0, 0);
  await expect(page.getByRole('img', { name: 'Recording' })).toHaveCSS('opacity', '1');
  await expect(page.getByText('Logging to picked.log')).toBeVisible();

  // The stop button's tooltip names the file and its size.
  const stop = page.getByRole('button', { name: 'Stop logging' });
  await stop.hover();
  await expect(stop).toHaveAttribute('title', 'Stop logging (/logs/picked.log, 2.0 KB)');
  await stop.click();
  await expect.poll(() => calls(page, 'log_stop')).toEqual([{ sessionId: 1 }]);
  await expect(page.getByText('Saved log to /logs/picked.log')).toBeVisible();
  await expect(page.getByText('Logging to picked.log')).toHaveCount(0);
  await expect(page.getByRole('img', { name: 'Recording' })).toHaveCount(0);

  await page.getByRole('button', { name: 'Show in folder' }).click();
  await expect.poll(() => calls(page, 'reveal_path')).toEqual([{ path: '/logs/picked.log' }]);
});

test('cancelling the dialog starts nothing', async ({ page }) => {
  await boot(page);
  await openTerminal(page);
  await setNextPath(page, null);

  await page.getByRole('button', { name: 'Start logging' }).click();
  await expect.poll(() => calls(page, 'pick_save_path')).toHaveLength(1);
  expect(await calls(page, 'log_start')).toEqual([]);
  await expect(page.getByRole('button', { name: 'Start logging' })).toBeEnabled();
});

test('timestamps follow the Settings switch', async ({ page }) => {
  await page.addInitScript(() => localStorage.setItem('omnyssh-log-timestamps', 'false'));
  await boot(page);
  await openTerminal(page);
  await page.getByRole('button', { name: 'Start logging' }).click();
  await expect.poll(async () => (await calls(page, 'log_start'))[0]?.timestamps).toBe(false);
});

test('a failed write stops the log with an error', async ({ page }) => {
  await boot(page);
  await openTerminal(page);
  await page.getByRole('button', { name: 'Start logging' }).click();
  await expect(page.getByText('Logging to picked.log')).toBeVisible();

  await page.evaluate(() =>
    (window as unknown as { __fire: (e: string, p: unknown) => void }).__fire('log-stopped', {
      sessionId: 1,
      path: '/logs/picked.log',
      bytes: 10,
      error: 'No space left on device'
    })
  );
  await expect(page.getByText('Logging to picked.log stopped: No space left on device')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Start logging' })).toBeVisible();
});

test('a split pane has the log and export buttons in its title bar', async ({ page }) => {
  await boot(page);
  await openTerminal(page);
  await page.getByRole('button', { name: 'Split right' }).click();
  await expect(page.getByRole('button', { name: 'Start logging' })).toHaveCount(2);
  await expect(page.getByRole('button', { name: 'Export contents' })).toHaveCount(2);

  await page.getByRole('button', { name: 'Start logging' }).first().click();
  // The title bar is always shown, so the pressed button itself is the indicator.
  await expect(page.getByRole('button', { name: 'Stop logging' })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: 'Start logging' })).toHaveCount(1);
});

test('export saves the scrollback and screen as text', async ({ page }) => {
  await boot(page);
  await openTerminal(page);
  await setNextPath(page, '/logs/web-1_export.txt');

  await page.getByRole('button', { name: 'Export contents' }).click();
  await expect.poll(() => calls(page, 'save_text_file')).toEqual([
    { path: '/logs/web-1_export.txt', text: 'omnyssh-ready>\n' }
  ]);
  const [pick] = await calls(page, 'pick_save_path');
  expect(pick).toEqual({ defaultName: expect.stringMatching(/^web-1_.*_export\.txt$/), kind: 'export' });
  await expect(page.getByText('Exported to /logs/web-1_export.txt')).toBeVisible();
});

test('a serial tab logs in the display it shows, and exports it', async ({ page }) => {
  await boot(page);
  await page.getByRole('button', { name: 'Serial', exact: true }).click();
  await page.getByRole('button', { name: 'Open', exact: true }).click();
  await expect(page.locator('.xterm')).toBeVisible();
  await page.evaluate(() => (window as unknown as { __sendSerial: (t: string) => void }).__sendSerial('hi\r\n'));
  await expect(page.locator('.xterm-rows')).toContainText('hi');

  await page.getByRole('button', { name: 'HEX', exact: true }).click();
  const log = page.getByRole('button', { name: 'Log', exact: true });
  await log.click();
  await expect
    .poll(() => calls(page, 'log_start'))
    .toEqual([
      {
        sessionId: 1,
        path: '/logs/picked.log',
        timestamps: true,
        mode: 'hex',
        header: expect.stringMatching(/^# OmnySSH log: COM3 \(COM3 · 115200 8N1\), started /)
      }
    ]);
  const [pick] = await calls(page, 'pick_save_path');
  expect(pick.defaultName).toMatch(/^COM3_\d{4}-\d\d-\d\d_\d\d-\d\d-\d\d\.log$/);
  await expect(log).toHaveAttribute('aria-pressed', 'true');

  // The log is still open on /logs/picked.log; the backend refuses to export into a
  // path it is currently logging to, so the export goes to a path of its own.
  await setNextPath(page, '/logs/com3-export.txt');
  await page.getByRole('button', { name: 'Export the output to a file' }).click();
  await expect.poll(() => calls(page, 'save_text_file')).toEqual([
    { path: '/logs/com3-export.txt', text: '68 69 0D 0A\n' }
  ]);

  await log.click();
  await expect.poll(() => calls(page, 'log_stop')).toEqual([{ sessionId: 1 }]);
  await expect(log).toHaveAttribute('aria-pressed', 'false');
});
