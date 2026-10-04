import { beforeEach, describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import { hostKeyPrompt, hostKeyQueue, hostKeyTitle, serverKeyFile, settleHostKey } from './hostKey';

const key = (fingerprint: string) => ({ keyType: 'ssh-ed25519', fingerprint });
const nas = {
  requestId: 1,
  hostName: 'nas',
  host: 'nas.example.com',
  port: 22,
  jumpFor: null,
  file: '/home/me/.ssh/known_hosts',
  saved: [key('SHA256:old')],
  offered: key('SHA256:new')
};
const bastion = { ...nas, requestId: 2, hostName: 'db', host: 'bastion.example.com', port: 2222, jumpFor: 'db' };

describe('host key question queue', () => {
  beforeEach(() => hostKeyQueue.set([]));

  it('shows the oldest waiting question, then the next once it is settled', () => {
    hostKeyQueue.set([nas, bastion]);
    expect(get(hostKeyPrompt)).toEqual(nas);
    settleHostKey(nas.requestId);
    expect(get(hostKeyPrompt)).toEqual(bastion);
    settleHostKey(bastion.requestId);
    expect(get(hostKeyPrompt)).toBeNull();
  });

  it('settling a question nobody shows changes nothing', () => {
    hostKeyQueue.set([nas]);
    settleHostKey(99);
    expect(get(hostKeyQueue)).toEqual([nas]);
  });
});

describe('hostKeyTitle', () => {
  it('names the host, its port off 22, and what a bastion was on the way to', () => {
    expect(hostKeyTitle(nas, false)).toBe('Host key changed: nas.example.com');
    expect(hostKeyTitle(bastion, false)).toBe('Host key changed: bastion.example.com:2222 (jump host for db)');
  });

  it('masks the host in streamer mode', () => {
    const title = hostKeyTitle(nas, true);
    expect(title.startsWith('Host key changed: ')).toBe(true);
    expect(title).not.toContain('nas.example.com');
  });
});

describe('serverKeyFile', () => {
  it('points at the server key file of the offered type', () => {
    expect(serverKeyFile('ssh-ed25519')).toBe('/etc/ssh/ssh_host_ed25519_key.pub');
    expect(serverKeyFile('ecdsa-sha2-nistp256')).toBe('/etc/ssh/ssh_host_ecdsa_key.pub');
    expect(serverKeyFile('ssh-rsa')).toBe('/etc/ssh/ssh_host_rsa_key.pub');
  });

  it('names no file for a type without a standard one', () => {
    expect(serverKeyFile('ssh-dss')).toBeNull();
    expect(serverKeyFile('sk-ssh-ed25519@openssh.com')).toBeNull();
  });
});
