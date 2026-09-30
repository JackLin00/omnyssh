// The serial line settings form, shared by the ad-hoc Serial dialog and the saved
// device editor, and the saved devices' dashboard helpers. Fields are strings, as the
// selects and inputs bind them; `lineConfig` / `deviceFromFields` validate and convert.
import type {
  FlowControlDto,
  LineEndingDto,
  ParityDto,
  SerialConfigDto,
  SerialDeviceDto,
  StopBitsDto
} from '$lib/bindings';

export interface SerialLineFields {
  port: string;
  baud: string;
  dataBits: string;
  parity: string;
  stopBits: string;
  flow: string;
  enter: string;
}

export interface SerialDeviceFields extends SerialLineFields {
  name: string;
  notes: string;
}

/** 115200 8N1, no flow control, Enter sends CR: what almost every MCU boots with. */
export function emptyLineFields(): SerialLineFields {
  return {
    port: '',
    baud: '115200',
    dataBits: '8',
    parity: 'none',
    stopBits: 'one',
    flow: 'none',
    enter: 'cr'
  };
}

export function emptyDeviceFields(): SerialDeviceFields {
  return { ...emptyLineFields(), name: '', notes: '' };
}

/** Digits only, 1..u32::MAX (the backend takes a u32). */
function parseBaud(baud: string): number | null {
  const t = baud.trim();
  if (!/^\d+$/.test(t)) return null;
  const n = Number(t);
  return n > 0 && n <= 0xffffffff ? n : null;
}

/** The config the fields describe, or null without a port or with a bad baud rate. */
export function lineConfig(f: SerialLineFields): SerialConfigDto | null {
  const baudRate = parseBaud(f.baud);
  if (f.port === '' || baudRate === null) return null;
  return {
    port: f.port,
    baudRate,
    dataBits: Number(f.dataBits),
    parity: f.parity as ParityDto,
    stopBits: f.stopBits as StopBitsDto,
    flowControl: f.flow as FlowControlDto
  };
}

export function fieldsFromDevice(d: SerialDeviceDto): SerialDeviceFields {
  return {
    name: d.name,
    port: d.config.port,
    baud: String(d.config.baudRate),
    dataBits: String(d.config.dataBits),
    parity: d.config.parity,
    stopBits: d.config.stopBits,
    flow: d.config.flowControl,
    enter: d.enter,
    notes: d.notes ?? ''
  };
}

export type DeviceResult = { ok: true; device: SerialDeviceDto } | { ok: false; error: string };

/** Validate the editor's fields. `taken` holds the other devices' names. */
export function deviceFromFields(f: SerialDeviceFields, taken: readonly string[]): DeviceResult {
  const name = f.name.trim();
  if (!name) return { ok: false, error: 'Name is required' };
  if (taken.includes(name)) {
    return { ok: false, error: `A serial device named "${name}" already exists` };
  }
  const config = lineConfig(f);
  if (!config) return { ok: false, error: 'Pick a port and a baud rate' };
  const notes = f.notes.trim();
  return {
    ok: true,
    device: { name, config, enter: f.enter as LineEndingDto, notes: notes || null }
  };
}

const PARITY_LETTER: Record<ParityDto, string> = { none: 'N', odd: 'O', even: 'E' };
const FLOW_LABEL: Record<FlowControlDto, string | null> = {
  none: null,
  software: 'XON/XOFF',
  hardware: 'RTS/CTS'
};

/** `COM3 · 115200 8N1`, plus the flow control when there is one. */
export function describeLine(c: SerialConfigDto): string {
  const frame = `${c.dataBits}${PARITY_LETTER[c.parity]}${c.stopBits === 'one' ? 1 : 2}`;
  const parts = [c.port, `${c.baudRate} ${frame}`];
  const flow = FLOW_LABEL[c.flowControl];
  if (flow) parts.push(flow);
  return parts.join(' · ');
}

/** The devices whose name, port or notes contain `query` (case-insensitive). */
export function filterSerialDevices(devices: SerialDeviceDto[], query: string): SerialDeviceDto[] {
  const q = query.trim().toLowerCase();
  if (!q) return devices;
  return devices.filter((d) =>
    [d.name, d.config.port, d.notes ?? ''].some((s) => s.toLowerCase().includes(q))
  );
}
