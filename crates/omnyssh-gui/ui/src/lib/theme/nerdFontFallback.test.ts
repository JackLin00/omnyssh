import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Nerd Font glyphs live in the Private Use Area, which no system monospace family
// carries. Two surfaces render raw remote output — xterm (its own `fontFamily`) and
// the chrome's `font-mono` utility, used by snippet results and SFTP previews — and
// their stacks are declared in different files, so they drift silently.
const UI = new URL('../../../', import.meta.url);

const NERD_FAMILIES = [
  'Symbols Nerd Font Mono',
  'Symbols Nerd Font',
  'MesloLGS NF',
  'JetBrainsMono Nerd Font Mono',
  'JetBrainsMono Nerd Font',
  'Hack Nerd Font Mono',
  'Hack Nerd Font',
  'FiraCode Nerd Font Mono',
  'FiraCode Nerd Font'
];

const STACKS: [string, string][] = [
  ['the terminal stack', 'src/lib/screens/TerminalPane.svelte'],
  ['the serial stack', 'src/lib/screens/SerialView.svelte'],
  ['the chrome font-mono stack', 'tailwind.config.ts']
];

const read = (relative: string): string =>
  readFileSync(fileURLToPath(new URL(relative, UI)), 'utf8');

// Prose names these families too, and an index taken over a comment proves nothing
// about the declaration — so comments come out before anything is measured. Cut from
// `//` to end of line rather than dropping the line, and never inside a `://` scheme.
const declarations = (relative: string): string =>
  read(relative)
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/(?<!:)\/\/[^\n]*/g, '');

// `ui-monospace` also ends in "monospace", so the generic has to be matched on its own.
const GENERIC = /(?<![-\w])monospace/;

// A closing quote pins the whole family name: without it `Symbols Nerd Font` is
// satisfied by the `Symbols Nerd Font Mono` entry, and deleting it goes unnoticed.
const entry = (family: string): RegExp => new RegExp(`${family}["']`);

describe('Nerd Font fallback', () => {
  for (const [label, file] of STACKS) {
    it(`${label} names every fallback family`, () => {
      const source = declarations(file);
      for (const family of NERD_FAMILIES) {
        expect(source, `${file} no longer falls back to ${family}`).toMatch(entry(family));
      }
    });

    it(`${label} keeps every patched family behind the generic monospace`, () => {
      // The load-bearing rule, and not the same as "behind the named system families":
      // the named ones are macOS/Windows-only, so on Linux anything ahead of the generic
      // becomes the Latin face and sizes the terminal cell from itself.
      const source = declarations(file);
      const generic = source.search(GENERIC);
      expect(generic, `${file} no longer names the generic monospace family`).toBeGreaterThan(-1);
      for (const family of NERD_FAMILIES) {
        expect(source.search(entry(family)), `${file} puts ${family} ahead of the generic`).toBeGreaterThan(
          generic
        );
      }
    });
  }
});
