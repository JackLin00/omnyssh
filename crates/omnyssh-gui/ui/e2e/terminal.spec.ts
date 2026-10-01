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

async function boot(
  page: Page,
  quickGroups: unknown[] = [],
  holdConnect = false
): Promise<void> {
  await page.addInitScript(
    ({ hosts, quickGroups, holdConnect }) => {
      let cbid = 0;
      let groups = quickGroups;
      const win = window as unknown as Record<string, unknown>;
      const listeners: Record<string, number[]> = {};
      // Per-channel outgoing index — the real Channel enforces message ordering.
      const chIndex: Record<number, number> = {};
      // Backend session id -> its output channel id, so terminal_write can echo.
      const sessionChannel: Record<number, number> = {};
      let nextSession = 0;
      // Held back with `holdConnect`, so a test can keep a pane "connecting" until it
      // calls `__releaseConnect`.
      const pendingPrompts: number[] = [];
      (win as { __releaseConnect?: () => void }).__releaseConnect = () => {
        for (const chId of pendingPrompts.splice(0)) sendToChannel(chId, 'omnyssh-ready> ');
      };

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
            case 'list_quick_commands':
              return Promise.resolve(groups);
            case 'save_quick_commands':
              groups = args.groups as unknown[];
              return Promise.resolve(null);
            case 'reload_hosts':
              return Promise.resolve(null);
            case 'terminal_open': {
              const chId = (args.onOutput as { id: number }).id;
              const sid = ++nextSession;
              sessionChannel[sid] = chId;
              // A shell prompt proves the streamed output renders + flips status to
              // connected; held back instead when a test wants the pane to stay
              // "connecting" until it releases it.
              if (holdConnect) pendingPrompts.push(chId);
              else setTimeout(() => sendToChannel(chId, 'omnyssh-ready> '), 0);
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
    { hosts: HOSTS, quickGroups, holdConnect }
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

test('Ctrl+Shift+F opens find, searches incrementally, re-presses reselect, and Escape closes it and returns focus', async ({
  page
}) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  await page.keyboard.press('Control+Shift+F');
  const bar = page.getByRole('search');
  await expect(bar).toBeVisible();

  const findInput = page.getByLabel('Find in terminal');
  await findInput.fill('ready');
  await expect(bar.getByText('1 / 1')).toBeVisible();

  // Pressing the chord again while the input still has focus keeps the bar open and
  // reselects its text, so typing replaces the query instead of appending to it.
  await page.keyboard.press('Control+Shift+F');
  await expect(bar).toBeVisible();
  await page.keyboard.type('x');
  await expect(findInput).toHaveValue('x');

  await page.keyboard.press('Escape');
  await expect(bar).toHaveCount(0);
  await expect(page.locator('.xterm-helper-textarea').first()).toBeFocused();
});

test("clicking another pane doesn't steal focus back from an open find bar", async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // Split right: pane A (left, the original session) and pane B (right, a new one),
  // which the split focuses.
  await page.getByTitle('Split right (Alt+Shift+=)').click();
  await expect(page.locator('.xterm')).toHaveCount(2);
  await expect(page.locator('.xterm-rows').nth(1)).toContainText('omnyssh-ready');

  // Focus pane A and open its find bar. `.xterm-screen` (not `.xterm-rows`, which it
  // overlays) is what actually receives pointer events.
  await page.locator('.xterm-screen').nth(0).click();
  await page.keyboard.press('Control+Shift+F');
  const findInput = page.getByLabel('Find in terminal');
  await expect(findInput).toBeVisible();

  // Click into pane B: its own becoming-focused effect must not reach across and pull
  // the keyboard out of pane A's still-open find input.
  await page.locator('.xterm-screen').nth(1).click();

  // Click back into pane A's find input and type: the keystrokes must land in the
  // input, not fall through to the terminal underneath it.
  await findInput.click();
  await page.keyboard.type('abc');
  await expect(findInput).toHaveValue('abc');
  expect(await writes(page)).toEqual([]);
});

test('dragging a pane by its title bar moves it, keeping its session and its keyboard', async ({
  page
}) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // Split right (Alt+Shift+=): pane 1 (left, the original session) and pane 2 (right).
  await page.keyboard.press('Alt+Shift+=');
  await expect(page.locator('[data-pane]')).toHaveCount(2);

  // The original session's xterm element, to prove the move doesn't remount it.
  const firstXterm = await page.locator('[data-pane="1"] .xterm').elementHandle();

  const titleBar1 = (await page.locator('[data-pane="1"] [data-titlebar]').boundingBox())!;
  const box2 = (await page.locator('[data-pane="2"]').boundingBox())!;

  // Press on the title bar's free area (left of its buttons), drag onto pane 2's top
  // edge, and release: pane 1 should land above pane 2.
  await page.mouse.move(titleBar1.x + 20, titleBar1.y + titleBar1.height / 2);
  await page.mouse.down();
  await page.mouse.move(box2.x + box2.width / 2, box2.y + box2.height / 2, { steps: 5 });
  await page.mouse.move(box2.x + box2.width / 2, box2.y + 10, { steps: 5 });
  await page.mouse.up();

  await expect(async () => {
    const box1After = (await page.locator('[data-pane="1"]').boundingBox())!;
    const box2After = (await page.locator('[data-pane="2"]').boundingBox())!;
    expect(Math.abs(box1After.x - box2After.x)).toBeLessThan(5);
    expect(box1After.y).toBeLessThan(box2After.y);
  }).toPass();

  // The moved pane's xterm element is the very same one — no remount, no dropped session.
  await expect(page.locator('[data-pane="1"] .xterm')).toHaveCount(1);
  expect(await firstXterm!.evaluate((el) => el.isConnected)).toBe(true);

  // The moved pane keeps the keyboard: typing lands on its own session (backend id 1),
  // not stranded on the title bar the drag started from.
  await page.keyboard.type('z');
  await expect.poll(() => writes(page)).toEqual([[122]]);
});

test("clicking a pane's title bar keeps the keyboard in its own terminal", async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // Split right: pane 2 (the new session) ends up focused.
  await page.keyboard.press('Alt+Shift+=');
  await expect(page.locator('[data-pane]')).toHaveCount(2);
  await expect(page.locator('.xterm-rows').nth(1)).toContainText('omnyssh-ready');

  // A plain click on pane 1's title bar (its free area, left of the buttons) only
  // switches focus, and must hand the keyboard to pane 1's own xterm — not strand it on
  // the title bar itself.
  await page.locator('[data-pane="1"] [data-titlebar]').click({ position: { x: 20, y: 12 } });
  // xterm reports each keystroke as its own onData event, so "hi" is two writes.
  await page.keyboard.type('hi');
  await expect.poll(() => writes(page)).toEqual([[104], [105]]);
});

test('Escape cancels a pane drag and leaves the layout as it was', async ({ page }) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  await page.keyboard.press('Alt+Shift+=');
  await expect(page.locator('[data-pane]')).toHaveCount(2);

  const titleBar1 = (await page.locator('[data-pane="1"] [data-titlebar]').boundingBox())!;
  const box2Before = (await page.locator('[data-pane="2"]').boundingBox())!;

  await page.mouse.move(titleBar1.x + 20, titleBar1.y + titleBar1.height / 2);
  await page.mouse.down();
  await page.mouse.move(box2Before.x + box2Before.width / 2, box2Before.y + 10, { steps: 5 });

  // The drag is actually under way before we cancel it.
  await expect(page.locator('[data-pane="1"].opacity-50')).toHaveCount(1);

  await page.keyboard.press('Escape');
  await page.mouse.up();

  await expect(async () => {
    const box1After = (await page.locator('[data-pane="1"]').boundingBox())!;
    const box2After = (await page.locator('[data-pane="2"]').boundingBox())!;
    // Still side by side: same y, pane 1 left of pane 2.
    expect(Math.abs(box1After.y - box2After.y)).toBeLessThan(5);
    expect(box1After.x).toBeLessThan(box2After.x);
  }).toPass();
});

test('a pane exiting mid-drag ends the move, with no dangling preview or leaked Escape listener', async ({
  page
}) => {
  await boot(page);
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  await page.keyboard.press('Alt+Shift+=');
  await expect(page.locator('[data-pane]')).toHaveCount(2);

  const titleBar1 = (await page.locator('[data-pane="1"] [data-titlebar]').boundingBox())!;
  const box2 = (await page.locator('[data-pane="2"]').boundingBox())!;

  await page.mouse.move(titleBar1.x + 20, titleBar1.y + titleBar1.height / 2);
  await page.mouse.down();
  await page.mouse.move(box2.x + box2.width / 2, box2.y + 10, { steps: 5 });

  // The drag is under way: the dragged pane is dimmed and a preview shows.
  await expect(page.locator('[data-pane="1"].opacity-50')).toHaveCount(1);
  await expect(page.locator('.border-accent')).toHaveCount(1);

  // Pane 1's session exits mid-drag: its title bar — the drag's pointer-capture target —
  // is removed from the DOM before the drag would otherwise end on its own.
  await page.evaluate(() => {
    (window as unknown as { __fireTerminalExited: (id: number) => void }).__fireTerminalExited(1);
  });
  await expect(page.locator('[data-pane]')).toHaveCount(1);

  // The move is cancelled on its own: no preview left dangling.
  await expect(page.locator('.border-accent')).toHaveCount(0);
  await page.mouse.up();

  // And no leaked capture-phase Escape listener: opening find in the surviving pane and
  // pressing Escape closes it on the very first press.
  await page.locator('.xterm-screen').first().click();
  await page.keyboard.press('Control+Shift+F');
  const findInput = page.getByLabel('Find in terminal');
  await expect(findInput).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(findInput).toHaveCount(0);
});

test.describe('quick command bar', () => {
  const QUICK_GROUPS = [
    {
      name: 'Ops',
      commands: [
        { label: 'hi', kind: 'text', payload: 'hi', ending: 'cr' },
        { label: 'bytes', kind: 'hex', payload: '41 42', ending: 'none' }
      ]
    }
  ];

  test('bar buttons are disabled while the pane is still connecting', async ({ page }) => {
    await boot(page, QUICK_GROUPS, true); // holdConnect: stays "connecting" until released
    await page.getByTitle('sh on web-1').click();

    const hiButton = page.getByRole('button', { name: 'hi', exact: true });
    await expect(hiButton).toBeVisible();
    await expect(hiButton).toBeDisabled();

    await page.evaluate(() => (window as unknown as { __releaseConnect: () => void }).__releaseConnect());
    await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');
    await expect(hiButton).toBeEnabled();
  });

  test('click, Shift+click, hex and Alt+1 send the right bytes, and a click leaves the keyboard with the terminal', async ({
    page
  }) => {
    await boot(page, QUICK_GROUPS);
    await page.getByTitle('sh on web-1').click();
    await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

    const hiButton = page.getByRole('button', { name: 'hi', exact: true });
    await expect(hiButton).toBeVisible();

    // A click sends the payload plus its ending, and gives the keyboard back to the
    // terminal rather than leaving it on the button: Alt+1 right after, with no
    // explicit focus, still reaches the session.
    await hiButton.click();
    await expect.poll(() => writes(page)).toEqual([[104, 105, 13]]);
    await page.keyboard.press('Alt+1');
    await expect.poll(() => writes(page)).toEqual([[104, 105, 13], [104, 105, 13]]);

    // Shift+click leaves a text command's ending off, and also returns the keyboard to
    // the terminal: pressing Enter now sends just its CR, not the whole command again
    // (which is what would happen if focus were stuck on the button).
    await hiButton.click({ modifiers: ['Shift'] });
    await expect.poll(() => writes(page)).toEqual([[104, 105, 13], [104, 105, 13], [104, 105]]);
    await page.keyboard.press('Enter');
    await expect.poll(() => writes(page)).toEqual([
      [104, 105, 13],
      [104, 105, 13],
      [104, 105],
      [13]
    ]);

    // A hex command sends its bytes.
    await page.getByRole('button', { name: 'bytes', exact: true }).click();
    await expect.poll(() => writes(page)).toEqual([
      [104, 105, 13],
      [104, 105, 13],
      [104, 105],
      [13],
      [65, 66]
    ]);
  });

  test('Alt+5, unbound to any command, reaches the shell instead of being swallowed', async ({
    page
  }) => {
    await boot(page, QUICK_GROUPS); // only 2 commands: slot 5 is empty
    await page.getByTitle('sh on web-1').click();
    await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

    await page.locator('.xterm-helper-textarea').focus();
    await page.keyboard.press('Alt+5');
    // xterm's own Alt handling: ESC followed by the character.
    await expect.poll(() => writes(page)).toEqual([[27, 53]]);
  });

  test('editing adds a command; collapsing hides the bar', async ({ page }) => {
    await boot(page, QUICK_GROUPS);
    await page.getByTitle('sh on web-1').click();
    await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

    const hiButton = page.getByRole('button', { name: 'hi', exact: true });
    await expect(hiButton).toBeVisible();

    // Edit mode: add a command, which is saved through the backend and shows up as a
    // new button.
    await page.getByRole('button', { name: 'Edit quick commands' }).click();
    await page.getByRole('button', { name: 'Add quick command' }).click();
    await page.getByLabel('Label').fill('list');
    await page.getByLabel('Text', { exact: true }).fill('ls');
    await page.getByRole('button', { name: 'Save', exact: true }).click();
    await expect(page.getByRole('button', { name: 'list', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Done editing' }).click();

    // Collapsing hides the bar but for the button that expands it again.
    await page.getByRole('button', { name: 'Hide quick commands' }).click();
    await expect(hiButton).toHaveCount(0);
    const expandButton = page.getByRole('button', { name: 'Show quick commands' });
    await expect(expandButton).toBeVisible();
    await expandButton.click();
    await expect(hiButton).toBeVisible();
  });
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

// Minimal stub for an ad-hoc serial tab (sidebar "Serial" spawner → `SerialConnect`):
// a port to auto-fill the dialog, `serial_open` capturing the output/exit channels
// (unused here — the device stays silent), and `serial_write` recording what the bar
// sends. No hosts: the dashboard isn't this test's concern.
async function bootSerial(page: Page, quickGroups: unknown[] = []): Promise<void> {
  await page.addInitScript(
    ({ quickGroups }) => {
      let cbid = 0;
      let groups = quickGroups;
      const win = window as unknown as Record<string, unknown>;

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
            case 'list_serial_devices':
              return Promise.resolve([]);
            case 'list_quick_commands':
              return Promise.resolve(groups);
            case 'save_quick_commands':
              groups = args.groups as unknown[];
              return Promise.resolve(null);
            case 'reload_hosts':
              return Promise.resolve(null);
            case 'serial_list_ports':
              return Promise.resolve([{ name: 'COM3', description: null }]);
            case 'serial_open':
              return Promise.resolve(1);
            case 'serial_write': {
              const { data } = args as { data: number[] };
              ((win.__serialWrites ??= []) as number[][]).push(data);
              return Promise.resolve(null);
            }
            case 'serial_close':
              return Promise.resolve(null);
            case 'plugin:event|listen':
              return Promise.resolve(++cbid);
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
    { quickGroups }
  );

  await page.goto('/');
}

const serialWrites = (page: Page) =>
  page.evaluate(() => (window as unknown as { __serialWrites?: number[][] }).__serialWrites ?? []);

test('a serial tab in monitor mode sends a quick command to the port', async ({ page }) => {
  await bootSerial(page, [
    { name: 'Ops', commands: [{ label: 'ping', kind: 'hex', payload: '55 AA', ending: 'none' }] }
  ]);

  await page.getByRole('button', { name: 'Serial', exact: true }).click();
  await expect(page.getByRole('dialog')).toBeVisible();
  // The port auto-fills from `serial_list_ports`; Monitor is the default mode.
  await page.getByRole('button', { name: 'Open', exact: true }).click();

  await expect(page.locator('.xterm')).toBeVisible();
  const pingButton = page.getByRole('button', { name: 'ping', exact: true });
  await expect(pingButton).toBeEnabled();

  await pingButton.click();
  await expect.poll(() => serialWrites(page)).toEqual([[0x55, 0xaa]]);
});
