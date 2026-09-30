import { describe, expect, it } from 'vitest';
import type { SerialDeviceDto } from '$lib/bindings';
import {
  describeLine,
  deviceFromFields,
  emptyDeviceFields,
  emptyLineFields,
  fieldsFromDevice,
  filterSerialDevices,
  lineConfig
} from './serialForm';

const device: SerialDeviceDto = {
  name: 'ESP32',
  config: {
    port: 'COM3',
    baudRate: 115200,
    dataBits: 8,
    parity: 'none',
    stopBits: 'one',
    flowControl: 'none'
  },
  enter: 'crlf',
  notes: 'bench board'
};

describe('lineConfig', () => {
  it('turns valid fields into a config', () => {
    expect(lineConfig({ ...emptyLineFields(), port: 'COM3' })).toEqual({
      port: 'COM3',
      baudRate: 115200,
      dataBits: 8,
      parity: 'none',
      stopBits: 'one',
      flowControl: 'none'
    });
  });

  it('rejects a missing port and a bad baud rate', () => {
    expect(lineConfig(emptyLineFields())).toBeNull();
    for (const baud of ['', '0', '115200abc', '1e5', '4294967296']) {
      expect(lineConfig({ ...emptyLineFields(), port: 'COM3', baud })).toBeNull();
    }
  });
});

describe('device fields', () => {
  it('round-trip a device', () => {
    expect(deviceFromFields(fieldsFromDevice(device), [])).toEqual({ ok: true, device });
  });

  it('need a name that no other device has', () => {
    const fields = { ...emptyDeviceFields(), port: 'COM3' };
    expect(deviceFromFields(fields, [])).toEqual({ ok: false, error: 'Name is required' });
    expect(deviceFromFields({ ...fields, name: ' ESP32 ' }, ['ESP32'])).toEqual({
      ok: false,
      error: 'A serial device named "ESP32" already exists'
    });
  });

  it('report a bad line setting', () => {
    expect(deviceFromFields({ ...emptyDeviceFields(), name: 'x' }, [])).toEqual({
      ok: false,
      error: 'Pick a port and a baud rate'
    });
  });

  it('drop blank notes', () => {
    const result = deviceFromFields({ ...fieldsFromDevice(device), notes: '  ' }, []);
    expect(result.ok && result.device.notes).toBeNull();
  });
});

describe('describeLine', () => {
  it('reads like 115200 8N1, with flow control when set', () => {
    expect(describeLine(device.config)).toBe('COM3 · 115200 8N1');
    expect(
      describeLine({ ...device.config, dataBits: 7, parity: 'even', stopBits: 'two', flowControl: 'hardware' })
    ).toBe('COM3 · 115200 7E2 · RTS/CTS');
    expect(describeLine({ ...device.config, parity: 'odd', flowControl: 'software' })).toBe(
      'COM3 · 115200 8O1 · XON/XOFF'
    );
  });
});

describe('filterSerialDevices', () => {
  it('matches name, port and notes, case-insensitively', () => {
    const other = { ...device, name: 'STM32', config: { ...device.config, port: 'COM7' }, notes: null };
    const all = [device, other];
    expect(filterSerialDevices(all, '')).toEqual(all);
    expect(filterSerialDevices(all, 'stm')).toEqual([other]);
    expect(filterSerialDevices(all, 'com3')).toEqual([device]);
    expect(filterSerialDevices(all, 'BENCH')).toEqual([device]);
  });
});
