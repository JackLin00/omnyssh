// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

const readText = vi.fn();
vi.mock('@tauri-apps/plugin-clipboard-manager', () => ({ readText }));
vi.mock('@tauri-apps/plugin-store', () => ({
  load: vi.fn(async () => {
    throw new Error('no tauri');
  })
}));

import { attachMouseClipboard } from './terminalClipboard';
import { copyOnSelect, rightClickPaste } from '$lib/stores/terminalPrefs';
import { lastError } from '$lib/stores/notifications';

/** The slice of xterm's Terminal the helper touches. */
function fakeTerm(selection = '') {
  return {
    hasSelection: vi.fn(() => selection !== ''),
    getSelection: vi.fn(() => selection),
    paste: vi.fn()
  };
}

const writeText = vi.fn();

beforeEach(() => {
  writeText.mockReset().mockResolvedValue(undefined);
  readText.mockReset();
  Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  copyOnSelect.set(true);
  rightClickPaste.set(true);
  lastError.set(null);
});

afterEach(() => {
  document.body.innerHTML = '';
});

function mouse(type: string, button = 0): MouseEvent {
  return new MouseEvent(type, { button, bubbles: true, cancelable: true });
}

describe('attachMouseClipboard', () => {
  it('a drag started in the terminal and released on document copies', () => {
    const el = document.createElement('div');
    const term = fakeTerm('hello');
    attachMouseClipboard(term as never, el, true);
    el.dispatchEvent(mouse('mousedown'));
    document.dispatchEvent(mouse('mouseup'));
    expect(writeText).toHaveBeenCalledWith('hello');
  });

  it('copies nothing without a selection, on another button, or when switched off', () => {
    const el = document.createElement('div');
    attachMouseClipboard(fakeTerm('') as never, el, true);
    el.dispatchEvent(mouse('mousedown'));
    document.dispatchEvent(mouse('mouseup'));
    const el2 = document.createElement('div');
    attachMouseClipboard(fakeTerm('x') as never, el2, true);
    el2.dispatchEvent(mouse('mousedown', 2));
    document.dispatchEvent(mouse('mouseup'));
    copyOnSelect.set(false);
    el2.dispatchEvent(mouse('mousedown'));
    document.dispatchEvent(mouse('mouseup'));
    expect(writeText).not.toHaveBeenCalled();
  });

  it('a mouseup on document without a preceding mousedown in the container copies nothing', () => {
    const el = document.createElement('div');
    attachMouseClipboard(fakeTerm('hello') as never, el, true);
    document.dispatchEvent(mouse('mouseup'));
    expect(writeText).not.toHaveBeenCalled();
  });

  it('reports a failed copy', async () => {
    writeText.mockRejectedValue(new Error('denied'));
    const el = document.createElement('div');
    attachMouseClipboard(fakeTerm('hello') as never, el, true);
    el.dispatchEvent(mouse('mousedown'));
    document.dispatchEvent(mouse('mouseup'));
    await vi.waitFor(() => expect(get(lastError)).toBe('Copy failed: denied'));
  });

  it('right-click pastes the clipboard through xterm and suppresses the menu', async () => {
    readText.mockResolvedValue('ls -l\n');
    const el = document.createElement('div');
    const term = fakeTerm();
    attachMouseClipboard(term as never, el, true);
    const e = mouse('contextmenu', 2);
    el.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(term.paste).toHaveBeenCalledWith('ls -l\n'));
  });

  it('right-click keeps the native menu when pasting is off or not allowed', () => {
    const el = document.createElement('div');
    attachMouseClipboard(fakeTerm() as never, el, false);
    const e1 = mouse('contextmenu', 2);
    el.dispatchEvent(e1);
    expect(e1.defaultPrevented).toBe(false);

    const el2 = document.createElement('div');
    attachMouseClipboard(fakeTerm() as never, el2, true);
    rightClickPaste.set(false);
    const e2 = mouse('contextmenu', 2);
    el2.dispatchEvent(e2);
    expect(e2.defaultPrevented).toBe(false);
    expect(readText).not.toHaveBeenCalled();
  });

  it('reports a failed paste', async () => {
    readText.mockRejectedValue(new Error('denied'));
    const el = document.createElement('div');
    attachMouseClipboard(fakeTerm() as never, el, true);
    el.dispatchEvent(mouse('contextmenu', 2));
    await vi.waitFor(() => expect(get(lastError)).toBe('Paste failed: denied'));
  });

  it('an empty clipboard pastes nothing', async () => {
    readText.mockResolvedValue('');
    const el = document.createElement('div');
    const term = fakeTerm();
    attachMouseClipboard(term as never, el, true);
    el.dispatchEvent(mouse('contextmenu', 2));
    await vi.waitFor(() => expect(readText).toHaveBeenCalled());
    expect(term.paste).not.toHaveBeenCalled();
  });

  it('the disposer removes both listeners', () => {
    readText.mockResolvedValue('x');
    const el = document.createElement('div');
    const dispose = attachMouseClipboard(fakeTerm('sel') as never, el, true);
    dispose();
    el.dispatchEvent(mouse('mousedown'));
    document.dispatchEvent(mouse('mouseup'));
    const e = mouse('contextmenu', 2);
    el.dispatchEvent(e);
    expect(writeText).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it('the disposer drops a pending document listener', () => {
    const el = document.createElement('div');
    const dispose = attachMouseClipboard(fakeTerm('sel') as never, el, true);
    el.dispatchEvent(mouse('mousedown'));
    dispose();
    document.dispatchEvent(mouse('mouseup'));
    expect(writeText).not.toHaveBeenCalled();
  });
});
