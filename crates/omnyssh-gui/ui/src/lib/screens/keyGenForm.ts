// The key generator editor's checks and the test panel's verdict, kept out of the
// component so they are unit-testable.
import type { HostDto, KeyGeneratorDto } from '$lib/bindings';

export interface GeneratorFields {
  name: string;
  code: string;
  credential: string;
}

export function emptyGeneratorFields(): GeneratorFields {
  return {
    name: '',
    code: 'async function generateKey(deviceInfo, address) {\n  // deviceInfo: { credential, mac, revision? }\n  return { privateKey: \'\' };\n}\n',
    credential: ''
  };
}

export type GeneratorResult = { ok: true; generator: KeyGeneratorDto } | { ok: false; error: string };

/** Name present and free (`taken`: the other generators' names), credential JSON or blank. */
export function generatorFromFields(f: GeneratorFields, taken: readonly string[]): GeneratorResult {
  const name = f.name.trim();
  if (!name) return { ok: false, error: 'Name is required' };
  if (taken.includes(name)) return { ok: false, error: `A key generator named "${name}" already exists` };
  if (f.credential.trim() !== '') {
    try {
      JSON.parse(f.credential);
    } catch {
      return { ok: false, error: 'The credential is not valid JSON' };
    }
  }
  return { ok: true, generator: { name, code: f.code, credential: f.credential.trim() } };
}

/** Why `raw` is not a device revision (1 to 4294967295, a host stores a u32), or null
 *  when it is one or blank (blank leaves it to the code). */
export function revisionError(raw: string): string | null {
  const r = raw.trim();
  if (r === '') return null;
  if (/^\d+$/.test(r) && Number(r) >= 1 && Number(r) <= 0xffffffff) return null;
  return `Revision must be a whole number from 1 to 4294967295, got '${r}'`;
}

/** Whether the code's own fingerprint agrees with the key's, `SHA256:` or not. */
export function fingerprintsAgree(fromKey: string, fromCode: string | undefined): boolean {
  if (fromCode === undefined) return true;
  const bare = (f: string) => f.trim().replace(/^SHA256:/, '').replace(/=+$/, '');
  return bare(fromKey) === bare(fromCode);
}

/** The hosts that name generator `name`. */
export function hostsUsing(hosts: readonly HostDto[], name: string): string[] {
  return hosts.filter((h) => h.keyGenerator === name).map((h) => h.name);
}
