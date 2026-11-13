import { describe, expect, it } from 'vitest';
import type { HostDto } from '$lib/bindings';
import {
  emptyGeneratorFields,
  fingerprintsAgree,
  generatorFromFields,
  hostsUsing,
  revisionError
} from './keyGenForm';

describe('generatorFromFields', () => {
  const fields = { name: ' vendor-a ', code: 'code', credential: ' {"master_key":"00ff"} ' };

  it('trims the name and the credential', () => {
    expect(generatorFromFields(fields, [])).toEqual({
      ok: true,
      generator: { name: 'vendor-a', code: 'code', credential: '{"master_key":"00ff"}' }
    });
  });

  it('wants a free name and a JSON credential, or none', () => {
    expect(generatorFromFields({ ...fields, name: ' ' }, [])).toEqual({ ok: false, error: 'Name is required' });
    expect(generatorFromFields(fields, ['vendor-a'])).toEqual({
      ok: false,
      error: 'A key generator named "vendor-a" already exists'
    });
    expect(generatorFromFields({ ...fields, credential: '{master_key: 1}' }, [])).toEqual({
      ok: false,
      error: 'The credential is not valid JSON'
    });
    expect(generatorFromFields({ ...fields, credential: '' }, []).ok).toBe(true);
  });

  it('starts a new generator from a template that defines generateKey', () => {
    expect(emptyGeneratorFields().code).toContain('async function generateKey(deviceInfo, address)');
  });
});

describe('fingerprintsAgree', () => {
  it('compares with or without the SHA256: prefix and base64 padding', () => {
    expect(fingerprintsAgree('SHA256:abc', 'SHA256:abc')).toBe(true);
    expect(fingerprintsAgree('SHA256:abc', 'abc=')).toBe(true);
    expect(fingerprintsAgree('SHA256:abc', 'SHA256:abd')).toBe(false);
    // Code that reports none has nothing to disagree with.
    expect(fingerprintsAgree('SHA256:abc', undefined)).toBe(true);
  });
});

describe('revisionError', () => {
  it('takes blank or 1 to 4294967295', () => {
    expect(revisionError(' ')).toBeNull();
    expect(revisionError(' 1 ')).toBeNull();
    expect(revisionError('4294967295')).toBeNull();
    for (const bad of ['0', '4294967296', '1.5', '-1', 'two']) {
      expect(revisionError(bad)).toBe(`Revision must be a whole number from 1 to 4294967295, got '${bad}'`);
    }
  });
});

describe('hostsUsing', () => {
  it('names the hosts that use a generator', () => {
    const hosts = [
      { name: 'cam-1', keyGenerator: 'vendor-a' },
      { name: 'web' },
      { name: 'cam-2', keyGenerator: 'vendor-a' }
    ] as HostDto[];
    expect(hostsUsing(hosts, 'vendor-a')).toEqual(['cam-1', 'cam-2']);
    expect(hostsUsing(hosts, 'other')).toEqual([]);
  });
});
