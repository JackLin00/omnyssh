import { describe, expect, it } from 'vitest';
import { searchDecorations, xtermTheme } from './terminalTheme';

// The xterm theme is derived from the brandbook tokens and pushed to every live
// terminal on toggle (tech-gui.md §5.1). Light and dark must differ on the surfaces
// a toggle re-paints, and both must define the full colour set xterm renders from —
// otherwise a toggle would leave a terminal half-themed.
const ANSI = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue',
  'brightMagenta', 'brightCyan', 'brightWhite'
] as const;

describe('xtermTheme', () => {
  it('light and dark differ on the load-bearing surfaces', () => {
    const light = xtermTheme('light');
    const dark = xtermTheme('dark');
    expect(dark.background).not.toBe(light.background);
    expect(dark.foreground).not.toBe(light.foreground);
    expect(dark.cursor).not.toBe(light.cursor);
  });

  it('both themes define background, foreground, cursor and all 16 ANSI colours', () => {
    for (const theme of [xtermTheme('light'), xtermTheme('dark')]) {
      expect(theme.background).toBeTruthy();
      expect(theme.foreground).toBeTruthy();
      expect(theme.cursor).toBeTruthy();
      for (const name of ANSI) expect(theme[name], `missing ${name}`).toBeTruthy();
    }
  });
});

describe('searchDecorations', () => {
  it('gives every highlight colour as #RRGGBB, which the search addon requires', () => {
    for (const t of ['dark', 'light'] as const) {
      const d = searchDecorations(t);
      for (const colour of [
        d.matchBackground,
        d.activeMatchBackground,
        d.matchOverviewRuler,
        d.activeMatchColorOverviewRuler
      ]) {
        expect(colour).toMatch(/^#[0-9a-f]{6}$/i);
      }
    }
  });
});
