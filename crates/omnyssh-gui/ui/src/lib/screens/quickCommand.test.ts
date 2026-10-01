import { describe, expect, it } from 'vitest';
import type { QuickCommandDto, QuickGroupDto } from '$lib/bindings';
import {
  addGroup,
  commandBytes,
  commandFromFields,
  deleteCommand,
  deleteGroup,
  emptyCommandFields,
  parseHex,
  renameGroup,
  upsertCommand
} from './quickCommand';

const text: QuickCommandDto = { label: 'disk', kind: 'text', payload: 'df -h', ending: 'cr' };
const hex: QuickCommandDto = { label: 'ping', kind: 'hex', payload: '55 aa 01', ending: 'crlf' };
const bytes = (...b: number[]) => Uint8Array.from(b);

describe('parseHex', () => {
  it('reads whole bytes with any spacing and case', () => {
    expect(parseHex('55 AA01')).toEqual(bytes(0x55, 0xaa, 0x01));
  });
  it('rejects odd digits, non-hex and nothing', () => {
    for (const s of ['', ' ', '5', '5G', '55 A']) expect(parseHex(s)).toBeNull();
  });
});

describe('commandBytes', () => {
  it('sends text as UTF-8 plus its ending', () => {
    expect(commandBytes(text, false)).toEqual(new TextEncoder().encode('df -h\r'));
    expect(commandBytes({ ...text, payload: '你', ending: 'lf' }, false)).toEqual(
      new TextEncoder().encode('你\n')
    );
  });
  it('drops the ending of a text command when only inserting', () => {
    expect(commandBytes(text, true)).toEqual(new TextEncoder().encode('df -h'));
  });
  it('sends hex bytes plus the ending either way', () => {
    expect(commandBytes(hex, false)).toEqual(bytes(0x55, 0xaa, 0x01, 0x0d, 0x0a));
    expect(commandBytes(hex, true)).toEqual(bytes(0x55, 0xaa, 0x01, 0x0d, 0x0a));
  });
  it('is null for bad hex and empty for nothing at all', () => {
    expect(commandBytes({ ...hex, payload: 'zz' }, false)).toBeNull();
    expect(commandBytes({ ...text, payload: '', ending: 'none' }, false)).toEqual(new Uint8Array());
  });
});

describe('commandFromFields', () => {
  it('trims the label and checks hex', () => {
    expect(commandFromFields({ ...emptyCommandFields(), label: ' ls ', payload: 'ls' })).toEqual({
      ok: true,
      command: { label: 'ls', kind: 'text', payload: 'ls', ending: 'cr' }
    });
    expect(commandFromFields(emptyCommandFields())).toEqual({ ok: false, error: 'Label is required' });
    expect(
      commandFromFields({ ...emptyCommandFields(), label: 'h', kind: 'hex', payload: '5' })
    ).toEqual({ ok: false, error: 'Hex must be whole bytes, like 55 AA 01' });
  });
});

describe('group edits', () => {
  const groups: QuickGroupDto[] = [{ name: 'A', commands: [text] }];

  it('add a uniquely named group, trimmed', () => {
    expect(addGroup(groups, ' B ')).toEqual({ ok: true, groups: [...groups, { name: 'B', commands: [] }] });
    expect(addGroup(groups, 'A')).toEqual({ ok: false, error: 'A group named "A" already exists' });
    expect(addGroup(groups, ' ')).toEqual({ ok: false, error: 'Name is required' });
  });

  it('rename a group, refusing a taken name', () => {
    const two = [...groups, { name: 'B', commands: [] }];
    expect(renameGroup(two, 'A', 'C')).toEqual({ ok: true, groups: [{ name: 'C', commands: [text] }, two[1]] });
    expect(renameGroup(two, 'A', 'B')).toEqual({ ok: false, error: 'A group named "B" already exists' });
    expect(renameGroup(two, 'A', 'A').ok).toBe(true);
  });

  it('delete a group', () => {
    expect(deleteGroup(groups, 'A')).toEqual([]);
  });

  it('add, replace and delete a command by index', () => {
    const added = upsertCommand(groups, 'A', null, hex);
    expect(added[0].commands).toEqual([text, hex]);
    const replaced = upsertCommand(added, 'A', 0, { ...text, label: 'du' });
    expect(replaced[0].commands[0].label).toBe('du');
    expect(deleteCommand(replaced, 'A', 1)[0].commands).toEqual([{ ...text, label: 'du' }]);
    // The input is never mutated.
    expect(groups[0].commands).toEqual([text]);
  });
});
