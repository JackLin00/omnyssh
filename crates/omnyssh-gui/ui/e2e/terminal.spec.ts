import { expect, test, type Page } from '@playwright/test';

// Terminal streaming vertical (tech-gui.md §3.1). e2e runs against the static SPA with
// Tauri absent, so we stub `__TAURI_INTERNALS__` at the boundary (§6.4). The stub is
// channel-aware: `terminal_open` captures the per-session output Channel and streams a
// prompt through it (proving raw output renders); `terminal_write` echoes a canned line
// on Enter (proving input round-trips). The host-first path (a Dashboard card's `sh`,
// no picker) is the load-bearing flow the stage requires.
const HOSTS = [
  { name: 'web-1', hostname: 'web-1.example.com', user: 'deploy', port: 22, tags: ['prod'], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false },
  { name: 'db-1', hostname: 'db-1.example.com', user: 'root', port: 22, tags: [], source: 'manual', hasKey: false, localForwards: [], tunnelAutostart: false, forwardAgent: false }
];

async function boot(page: Page): Promise<void> {
  await page.addInitScript(
    ({ hosts }) => {
      let cbid = 0;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      // Per-channel outgoing index — the real Channel enforces message ordering.
      const chIndex: Record<number, number> = {};
      // Backend session id -> its output channel id, so terminal_write can echo.
      const sessionChannel: Record<number, number> = {};
      let nextSession = 0;

      function sendToChannel(chId: number, text: string): void {
        const cb = win[`__cb${chId}`] as ((m: unknown) => void) | undefined;
        const index = chIndex[chId] ?? 0;
        chIndex[chId] = index + 1;
        // The raw path delivers an ArrayBuffer; mirror that so xterm's Uint8Array wrap works.
        cb?.({ message: new TextEncoder().encode(text).buffer, index });
      }

      function fireEvent(event: string, payload: unknown): void {
        for (const id of listeners[event] ?? []) {
          const cb = win[`__cb${id}`] as ((e: unknown) => void) | undefined;
          cb?.({ event, id, payload });
        }
      }
      // Lets a test simulate the remote shell exiting for a given backend session id.
      (win as { __fireTerminalExited?: (sessionId: number) => void }).__fireTerminalExited = (
        sessionId
      ) => fireEvent('terminal-exited', { sessionId });

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
              return Promise.resolve(hosts);
            // e2e's Desktop Chrome UA contains "Windows", so the dashboard's serial
            // devices load runs here too; an empty list keeps it a no-op.
            case 'list_serial_devices':
              return Promise.resolve([]);
            case 'reload_hosts':
              return Promise.resolve(null);
            case 'terminal_open': {
              const chId = (args.onOutput as { id: number }).id;
              const sid = ++nextSession;
              sessionChannel[sid] = chId;
              // A shell prompt proves the streamed output renders + flips status to connected.
              setTimeout(() => sendToChannel(chId, 'omnyssh-ready> '), 0);
              return Promise.resolve(sid);
            }
            case 'terminal_write': {
              const { sessionId, data } = args as { sessionId: number; data: number[] };
              // Every byte the shell would get, for tests that assert what a key sent.
              ((win.__writes ??= []) as number[][]).push(data);
              const chId = sessionChannel[sessionId];
              // Echo a canned result once Enter (\r == 13) arrives, so output is assertable.
              if (chId != null && data.includes(13)) {
                setTimeout(() => sendToChannel(chId, '\r\nRESULT-OK\r\n'), 0);
              }
              return Promise.resolve(null);
            }
            case 'terminal_paste':
              win.__pasted = ((win.__pasted as number | undefined) ?? 0) + 1;
              return Promise.resolve(null);
            // The keyboard-shortcut paste (default Ctrl+Shift+V) reads through this
            // plugin, the same path as right-click paste; empty unless a test sets it.
            case 'plugin:clipboard-manager|read_text':
              return Promise.resolve((win.__clipboardText as string | undefined) ?? '');
            case 'terminal_resize':
            case 'terminal_close':
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
    { hosts: HOSTS }
  );

  await page.goto('/');
  // The status-bar total confirms the app booted and `list_hosts` resolved.
  await expect(page.getByText('2 hosts')).toBeVisible();
}

test('host-first: spawn a terminal from a card, run a command, see output, then close', async ({
  page
}) => {
  await boot(page);

  // Host-first spawn — a Dashboard card's `sh`, no picker (tech-gui.md §2, §3.1).
  await page.getByTitle('sh on web-1').click();

  // The tab row appears and the terminal renders the streamed prompt.
  await expect(page.getByRole('button', { name: 'web-1 · terminal', exact: true })).toBeVisible();
  await expect(page.locator('.xterm')).toBeVisible();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // Run a command: focus the terminal input, type, press Enter -> canned output streams back.
  await page.locator('.xterm-helper-textarea').focus();
  await page.keyboard.type('hi');
  await page.keyboard.press('Enter');
  await expect(page.locator('.xterm-rows')).toContainText('RESULT-OK');

  // Closing the tab tears the terminal down.
  await page.getByRole('button', { name: 'Close web-1', exact: true }).click();
  await expect(page.locator('.xterm')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'web-1 · terminal', exact: true })).toHaveCount(0);
});

test('action-first: the Terminal spawner opens the host picker, then a live terminal', async ({
  page
}) => {
  await boot(page);

  // Action-first spawn — the sidebar Terminal spawner opens the host picker (§2).
  await page.getByRole('button', { name: 'Terminal', exact: true }).click();
  await page.getByRole('dialog').getByText('web-1', { exact: true }).click();

  await expect(page.getByRole('button', { name: 'web-1 · terminal', exact: true })).toBeVisible();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
});

test('toggling the theme re-themes a live terminal (§5.1)', async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // xterm paints its scrollable viewport inline with theme.background; find that
  // element's computed colour rather than assume a class (robust across versions).
  const paintedBg = () =>
    page.evaluate(() => {
      const root = document.querySelector('.xterm');
      const els = root ? Array.from(root.querySelectorAll<HTMLElement>('*')) : [];
      const painted = els.find((el) => el.style.backgroundColor);
      return painted ? getComputedStyle(painted).backgroundColor : '';
    });

  // App defaults to dark → the dark surface (#212121).
  await expect.poll(paintedBg).toBe('rgb(33, 33, 33)');

  // The #1 theme-regression guard: flipping the store re-themes the OPEN terminal.
  await page.getByTitle('Switch to light theme').click();
  await expect.poll(paintedBg).toBe('rgb(255, 255, 255)');
});

test('a remote exit (terminal-exited) tears the tab down', async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.getByRole('button', { name: 'web-1 · terminal', exact: true })).toBeVisible();
  await expect(page.locator('.xterm')).toBeVisible();

  // The remote shell exits: the backend emits terminal-exited for session id 1.
  await page.evaluate(() => {
    (window as unknown as { __fireTerminalExited: (id: number) => void }).__fireTerminalExited(1);
  });

  await expect(page.getByRole('button', { name: 'web-1 · terminal', exact: true })).toHaveCount(0);
  await expect(page.locator('.xterm')).toHaveCount(0);
});

test('Ctrl+Shift+F opens find, searches incrementally, and Escape closes it', async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  await page.keyboard.press('Control+Shift+F');
  const bar = page.getByRole('search');
  await expect(bar).toBeVisible();

  await page.getByLabel('Find in terminal').fill('ready');
  // Incremental search may not have a current match selected yet, showing just the
  // total; Enter steps to (and selects) the first one.
  if (!(await bar.getByText('1 / 1').count())) {
    await page.keyboard.press('Enter');
  }
  await expect(bar.getByText('1 / 1')).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(bar).toHaveCount(0);
});

// Windows and Linux copy with Ctrl+Shift+C. The Desktop Chrome device reports a Windows
// user agent, so this is the path those platforms take; the clipboard is stubbed at the
// boundary like the IPC, which also keeps parallel runs apart.
async function bootWithClipboard(page: Page, copyOnSelect = false): Promise<void> {
  await page.addInitScript((copyOnSelect) => {
    const win = window as unknown as { __copied: string[] };
    win.__copied = [];
    // Most of these tests exercise the Ctrl+Shift+C chord alone, so copy-on-select (which
    // copies the double-click selection by itself) is switched off unless asked for.
    if (!copyOnSelect) localStorage.setItem('omnyssh-copy-on-select', 'false');
    navigator.clipboard.writeText = (text: string) => {
      win.__copied.push(text);
      return Promise.resolve();
    };
  }, copyOnSelect);
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
}

const copied = (page: Page) =>
  page.evaluate(() => (window as unknown as { __copied: string[] }).__copied);
const writes = (page: Page) =>
  page.evaluate(() => (window as unknown as { __writes?: number[][] }).__writes ?? []);

/** Double-clicks the first word of the first row, as a user selects it. */
async function selectPrompt(page: Page): Promise<void> {
  const row = (await page.locator('.xterm-rows > div').first().boundingBox())!;
  await page.mouse.dblclick(row.x + 20, row.y + row.height / 2);
}

test('Ctrl+Shift+C copies the selection and sends the shell nothing', async ({ page }) => {
  await bootWithClipboard(page);
  await selectPrompt(page);

  await page.keyboard.press('Control+Shift+C');
  await expect.poll(() => copied(page)).toEqual(['omnyssh-ready>']);
  expect(await writes(page)).toEqual([]);

  // Bare Ctrl+C stays the interrupt, selection or not.
  await page.keyboard.press('Control+C');
  await expect.poll(() => writes(page)).toEqual([[3]]);
  expect(await copied(page)).toEqual(['omnyssh-ready>']);

  // Ctrl+Shift+V is the webview's own paste; xterm must not turn it into ^V or a V.
  await page.keyboard.press('Control+Shift+V');
  expect(await writes(page)).toEqual([[3]]);
});

test('copy-on-select copies a double-click selection with no key pressed', async ({ page }) => {
  await bootWithClipboard(page, true);
  await selectPrompt(page);

  await expect.poll(() => copied(page)).toEqual(['omnyssh-ready>']);
  expect(await writes(page)).toEqual([]);
});

test('Ctrl+Shift+C with nothing selected copies nothing', async ({ page }) => {
  await bootWithClipboard(page);
  await page.locator('.xterm-helper-textarea').focus();

  await page.keyboard.press('Control+Shift+C');
  expect(await copied(page)).toEqual([]);
  expect(await writes(page)).toEqual([]);
});

// WebKitGTK under a Russian layout reports keyCode 0 for letter keys, which is also
// what a synthetic keydown carries unless told otherwise — so this is the key event
// xterm gets there: without the fallback, Ctrl+C would send nothing at all.
test('under a non-Latin layout Ctrl+C still interrupts and Ctrl+Shift+V still pastes', async ({
  page
}) => {
  await bootWithClipboard(page);
  const press = (code: string, shiftKey: boolean, keyCode = 0) =>
    page.locator('.xterm-helper-textarea').evaluate(
      (el, init) => {
        el.dispatchEvent(new KeyboardEvent('keydown', { ...init, ctrlKey: true, bubbles: true }));
      },
      { key: '\u0441', code, shiftKey, keyCode }
    );

  await press('KeyC', false);
  await expect.poll(() => writes(page)).toEqual([[3]]);

  // Where the webview does report the key (WebView2 under the same layout), xterm
  // sends ^C itself and the fallback stays out: one ^C, not two.
  await press('KeyC', false, 67);
  await expect.poll(() => writes(page)).toEqual([[3], [3]]);

  // The default paste chord (Ctrl+Shift+V) matches on the physical key too, so with
  // paste still bound to it, this pastes through the same clipboard-manager path as
  // right-click. Only when paste is unbound or rebound does the WebKitGTK fallback
  // below take over (see the next test).
  await page.evaluate(() => {
    (window as unknown as { __clipboardText: string }).__clipboardText = 'hi';
  });
  await press('KeyV', true);
  await expect.poll(() => writes(page)).toEqual([[3], [3], [104, 105]]);
});

test('under a non-Latin layout, an unbound paste still falls back to the webview paste', async ({
  page
}) => {
  await page.addInitScript(() => {
    localStorage.setItem('omnyssh-terminal-shortcuts', JSON.stringify({ paste: null }));
  });
  await bootWithClipboard(page);

  await page.locator('.xterm-helper-textarea').evaluate((el) => {
    el.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'с',
        code: 'KeyV',
        keyCode: 0,
        ctrlKey: true,
        shiftKey: true,
        bubbles: true
      })
    );
  });
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __pasted?: number }).__pasted))
    .toBe(1);
});

test.describe('on macOS', () => {
  test.use({
    userAgent:
      'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)'
  });

  test('Ctrl+Shift+C is left alone — Cmd+C copies there', async ({ page }) => {
    await bootWithClipboard(page);
    await selectPrompt(page);

    await page.keyboard.press('Control+Shift+C');
    expect(await copied(page)).toEqual([]);
  });
});
