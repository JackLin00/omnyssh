import { expect, test, type Page } from '@playwright/test';

// Saved serial devices on the dashboard (Windows only — the Desktop Chrome UA e2e
// runs under reports Windows, so the cards are exercised here too). Tauri is absent,
// so we stub `__TAURI_INTERNALS__` at the boundary the way hosts.spec.ts and
// snippets.spec.ts do: `save`/`delete` mutate an in-memory list that
// `list_serial_devices` reads back, so a save/delete round-trips into the grid
// exactly as the real backend would drive it.
type Device = {
  name: string;
  config: {
    port: string;
    baudRate: number;
    dataBits: number;
    parity: string;
    stopBits: string;
    flowControl: string;
  };
  enter: string;
  notes: string | null;
};

const PORTS = [{ name: 'COM3', description: 'USB-SERIAL CH340' }];

async function boot(page: Page, devices: Device[] = []): Promise<void> {
  await page.addInitScript(
    ({ ports, devices }) => {
      let cbid = 0;
      const state: { devices: Device[] } = { devices: devices.map((d) => ({ ...d })) };
      const win = window as unknown as Record<string, unknown>;

      (win as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {
        invoke: (cmd: string, args: Record<string, unknown>) => {
          switch (cmd) {
            case 'list_hosts':
              return Promise.resolve([]);
            case 'reload_hosts':
              return Promise.resolve(null);
            case 'serial_list_ports':
              return Promise.resolve(ports);
            case 'list_serial_devices':
              return Promise.resolve([...state.devices]);
            case 'list_quick_commands':
              return Promise.resolve([]);
            case 'save_quick_commands':
              return Promise.resolve(null);
            case 'save_serial_device': {
              const d = args.device as Device;
              const i = state.devices.findIndex((x) => x.name === d.name);
              if (i >= 0) state.devices[i] = d;
              else state.devices.push(d);
              return Promise.resolve(null);
            }
            case 'delete_serial_device':
              state.devices = state.devices.filter((x) => x.name !== (args.name as string));
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
        }
      };
    },
    { ports: PORTS, devices }
  );
  await page.goto('/');
  // The header button is the boot marker: it only renders on Windows, once the
  // dashboard has mounted.
  await expect(page.getByRole('button', { name: 'Add serial' })).toBeVisible();
}

test('adds a serial device and it appears as a card', async ({ page }) => {
  await boot(page);

  await page.getByRole('button', { name: 'Add serial' }).click();
  const editor = page.getByRole('dialog', { name: 'Add serial device' });
  await expect(editor).toBeVisible();

  // The port select (the dialog's first <select>) fills in from the first scanned
  // port once it resolves. It has no standalone accessible name of its own: the
  // wrapping <label> computes its name from the selected option's own text too, so
  // position, not getByLabel, is what pins it down here.
  await expect(editor.locator('select').first()).toHaveValue('COM3');
  await editor.getByLabel('Name', { exact: true }).fill('ESP32 devkit');
  await editor.getByRole('button', { name: 'Save' }).click();

  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByText('ESP32 devkit', { exact: true })).toBeVisible();
  await expect(page.getByText('COM3 · 115200 8N1')).toBeVisible();
});

test('search filters serial devices by port', async ({ page }) => {
  await boot(page, [
    { name: 'ESP32', config: { port: 'COM3', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' }, enter: 'cr', notes: null },
    { name: 'STM32', config: { port: 'COM7', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' }, enter: 'cr', notes: null }
  ]);
  await expect(page.getByText('ESP32', { exact: true })).toBeVisible();
  await expect(page.getByText('STM32', { exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Search hosts and devices' }).click();
  await page.getByPlaceholder('Search hosts and devices…').fill('com3');

  await expect(page.getByText('ESP32', { exact: true })).toBeVisible();
  await expect(page.getByText('STM32', { exact: true })).toHaveCount(0);
});

test('deletes a serial device after confirmation', async ({ page }) => {
  await boot(page, [
    { name: 'ESP32', config: { port: 'COM3', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' }, enter: 'cr', notes: null }
  ]);
  await expect(page.getByText('ESP32', { exact: true })).toBeVisible();

  await page.getByRole('button', { name: 'Delete ESP32' }).click();
  const confirm = page.getByRole('dialog', { name: 'Delete serial device' });
  await expect(confirm).toBeVisible();
  await confirm.getByRole('button', { name: 'Delete', exact: true }).click();

  await expect(page.getByRole('dialog')).toHaveCount(0);
  await expect(page.getByText('ESP32', { exact: true })).toHaveCount(0);
});

test('renders duplicate-named serial devices without crashing (a hand-edited serial.toml can hold two)', async ({
  page
}) => {
  await boot(page, [
    { name: 'dup', config: { port: 'COM3', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' }, enter: 'cr', notes: null },
    { name: 'dup', config: { port: 'COM4', baudRate: 115200, dataBits: 8, parity: 'none', stopBits: 'one', flowControl: 'none' }, enter: 'cr', notes: null }
  ]);

  // A name-keyed each would throw each_key_duplicate and blank the screen; both
  // cards, distinguishable by their port, must render.
  await expect(page.getByText('COM3 · 115200 8N1')).toBeVisible();
  await expect(page.getByText('COM4 · 115200 8N1')).toBeVisible();
});
