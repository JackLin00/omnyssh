import { expect, test, type Page } from '@playwright/test';

// Terminal colour schemes (Settings → Terminal colors). Reuses terminal.spec.ts's
// channel-aware `__TAURI_INTERNALS__` stub: `terminal_open` streams a prompt so a live
// terminal actually renders, proving a scheme picked in Settings repaints it immediately,
// and that the pick survives a reload (localStorage mirror, §5.1-style persistence).
const HOSTS = [
  { name: 'web-1', hostname: 'web-1.example.com', user: 'deploy', port: 22, tags: [], source: 'manual', hasKey: true, localForwards: [], tunnelAutostart: false, forwardAgent: false }
];

async function boot(page: Page): Promise<void> {
  await page.addInitScript(({ hosts }) => {
    let cbid = 0;
    const win = window as unknown as Record<string, unknown>;
    const listeners: Record<string, number[]> = {};
    const chIndex: Record<number, number> = {};
    let nextSession = 0;

    function sendToChannel(chId: number, text: string): void {
      const cb = win[`__cb${chId}`] as ((m: unknown) => void) | undefined;
      const index = chIndex[chId] ?? 0;
      chIndex[chId] = index + 1;
      cb?.({ message: new TextEncoder().encode(text).buffer, index });
    }

    (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
      invoke: (cmd: string, args: Record<string, unknown>) => {
        switch (cmd) {
          case 'list_hosts':
            return Promise.resolve(hosts);
          case 'list_serial_devices':
            return Promise.resolve([]);
          case 'list_quick_commands':
            return Promise.resolve([]);
          case 'reload_hosts':
            return Promise.resolve(null);
          case 'terminal_open': {
            const chId = (args.onOutput as { id: number }).id;
            const sid = ++nextSession;
            setTimeout(() => sendToChannel(chId, 'omnyssh-ready> '), 0);
            return Promise.resolve(sid);
          }
          case 'terminal_resize':
          case 'terminal_close':
            return Promise.resolve(null);
          case 'set_tray_behavior':
            return Promise.resolve({ available: true, minimize: true });
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
  }, { hosts: HOSTS });

  await page.goto('/');
  await expect(page.getByText('web-1', { exact: true })).toBeVisible();
}

// xterm paints its scrollable viewport inline with theme.background; find that element's
// computed colour rather than assume a class (robust across xterm versions, see terminal.spec.ts).
const paintedBg = (page: Page) =>
  page.evaluate(() => {
    const root = document.querySelector('.xterm');
    const els = root ? Array.from(root.querySelectorAll<HTMLElement>('*')) : [];
    const painted = els.find((el) => el.style.backgroundColor);
    return painted ? getComputedStyle(painted).backgroundColor : '';
  });

test('picking Catppuccin in Settings repaints a live terminal, and the pick survives a reload', async ({
  page
}) => {
  await boot(page);

  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings' })).toBeVisible();

  const catppuccin = page.getByRole('button', { name: 'Catppuccin' });
  await catppuccin.click();
  await expect(catppuccin).toHaveAttribute('aria-pressed', 'true');

  // Back to the Dashboard to open a live terminal; it must pick up the chosen scheme.
  await page.getByRole('button', { name: 'Dashboard', exact: true }).click();
  await page.getByTitle('sh on web-1').click();
  await expect(page.locator('.xterm-rows')).toContainText('omnyssh-ready');

  // Catppuccin Mocha's background, #1e1e2e.
  await expect.poll(() => paintedBg(page)).toBe('rgb(30, 30, 46)');

  // The pick persists across a reload.
  await page.reload();
  await expect(page.getByText('web-1', { exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Settings' }).click();
  await expect(page.getByRole('button', { name: 'Catppuccin' })).toHaveAttribute(
    'aria-pressed',
    'true'
  );
});
