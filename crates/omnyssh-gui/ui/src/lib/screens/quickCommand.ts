// The quick command bar's pure parts: turning a command into the bytes it sends, the
// editor's validation, and immutable edits of the groups.
import type { EndingDto, PayloadKindDto, QuickCommandDto, QuickGroupDto } from '$lib/bindings';

const ENDINGS: Record<EndingDto, number[]> = {
  none: [],
  cr: [0x0d],
  lf: [0x0a],
  crlf: [0x0d, 0x0a]
};

/** `55 AA01` → its bytes; null unless it is at least one whole byte of hex. */
export function parseHex(s: string): Uint8Array | null {
  const digits = s.replace(/\s+/g, '');
  if (digits === '' || digits.length % 2 !== 0 || !/^[0-9a-f]+$/i.test(digits)) return null;
  return Uint8Array.from(digits.match(/../g)!, (pair) => parseInt(pair, 16));
}

function concat(a: Uint8Array, b: number[]): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a);
  out.set(b, a.length);
  return out;
}

/** What a click sends: the payload, then the ending — which an insert-only click
 *  (Shift) leaves off a text command. Null when the hex is unreadable. */
export function commandBytes(cmd: QuickCommandDto, insertOnly: boolean): Uint8Array | null {
  if (cmd.kind === 'hex') {
    const payload = parseHex(cmd.payload);
    return payload ? concat(payload, ENDINGS[cmd.ending]) : null;
  }
  const payload = new TextEncoder().encode(cmd.payload);
  return insertOnly ? payload : concat(payload, ENDINGS[cmd.ending]);
}

export interface CommandFields {
  label: string;
  kind: string;
  payload: string;
  ending: string;
}

export function emptyCommandFields(): CommandFields {
  return { label: '', kind: 'text', payload: '', ending: 'cr' };
}

export function fieldsFromCommand(c: QuickCommandDto): CommandFields {
  return { ...c };
}

export type CommandResult = { ok: true; command: QuickCommandDto } | { ok: false; error: string };

export function commandFromFields(f: CommandFields): CommandResult {
  const label = f.label.trim();
  if (!label) return { ok: false, error: 'Label is required' };
  if (f.kind === 'hex' && !parseHex(f.payload)) {
    return { ok: false, error: 'Hex must be whole bytes, like 55 AA 01' };
  }
  return {
    ok: true,
    command: {
      label,
      kind: f.kind as PayloadKindDto,
      payload: f.payload,
      ending: f.ending as EndingDto
    }
  };
}

export type GroupsResult = { ok: true; groups: QuickGroupDto[] } | { ok: false; error: string };

function checkName(groups: QuickGroupDto[], raw: string, except?: string): string | { error: string } {
  const name = raw.trim();
  if (!name) return { error: 'Name is required' };
  if (groups.some((g) => g.name === name && g.name !== except)) {
    return { error: `A group named "${name}" already exists` };
  }
  return name;
}

export function addGroup(groups: QuickGroupDto[], raw: string): GroupsResult {
  const name = checkName(groups, raw);
  if (typeof name !== 'string') return { ok: false, ...name };
  return { ok: true, groups: [...groups, { name, commands: [] }] };
}

export function renameGroup(groups: QuickGroupDto[], from: string, raw: string): GroupsResult {
  const name = checkName(groups, raw, from);
  if (typeof name !== 'string') return { ok: false, ...name };
  return { ok: true, groups: groups.map((g) => (g.name === from ? { ...g, name } : g)) };
}

export function deleteGroup(groups: QuickGroupDto[], name: string): QuickGroupDto[] {
  return groups.filter((g) => g.name !== name);
}

/** Add `cmd` to group `name` (index null) or replace its command at `index`. */
export function upsertCommand(
  groups: QuickGroupDto[],
  name: string,
  index: number | null,
  cmd: QuickCommandDto
): QuickGroupDto[] {
  return groups.map((g) => {
    if (g.name !== name) return g;
    const commands = [...g.commands];
    if (index === null) commands.push(cmd);
    else commands[index] = cmd;
    return { ...g, commands };
  });
}

export function deleteCommand(groups: QuickGroupDto[], name: string, index: number): QuickGroupDto[] {
  return groups.map((g) =>
    g.name === name ? { ...g, commands: g.commands.filter((_, i) => i !== index) } : g
  );
}
