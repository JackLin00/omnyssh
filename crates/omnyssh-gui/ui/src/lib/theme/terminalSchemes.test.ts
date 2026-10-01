import { describe, expect, it } from 'vitest';
import { isDarkBackground, resolveScheme, TERMINAL_SCHEMES } from './terminalSchemes';
import { xtermTheme } from './terminalTheme';

const ANSI = [
  'black', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'white',
  'brightBlack', 'brightRed', 'brightGreen', 'brightYellow', 'brightBlue', 'brightMagenta', 'brightCyan', 'brightWhite'
] as const;

describe('terminal schemes', () => {
  it('lists the ten schemes, the app\'s own first', () => {
    expect(TERMINAL_SCHEMES.map((s) => s.id)).toEqual([
      'omnyssh', 'catppuccin', 'tokyonight', 'onehalf', 'github', 'gruvbox', 'solarized', 'rosepine', 'nord', 'dracula'
    ]);
  });

  it('gives every variant all twenty colours as #rrggbb', () => {
    for (const s of TERMINAL_SCHEMES.filter((x) => x.id !== 'omnyssh')) {
      for (const v of [s.dark, s.light].filter(Boolean)) {
        for (const key of ['background', 'foreground', 'cursor', 'selectionBackground', ...ANSI] as const) {
          expect(v![key], `${s.id}.${key}`).toMatch(/^#[0-9a-f]{6}$/i);
        }
      }
    }
  });

  it('follows the app theme, and falls back to dark where there is no light variant', () => {
    expect(resolveScheme('omnyssh', 'dark')).toEqual(xtermTheme('dark'));
    expect(resolveScheme('omnyssh', 'light')).toEqual(xtermTheme('light'));
    const cat = TERMINAL_SCHEMES.find((s) => s.id === 'catppuccin')!;
    expect(resolveScheme('catppuccin', 'light')).toEqual(cat.light);
    expect(resolveScheme('catppuccin', 'dark')).toEqual(cat.dark);
    const nord = TERMINAL_SCHEMES.find((s) => s.id === 'nord')!;
    expect(nord.light).toBeNull();
    expect(resolveScheme('nord', 'light')).toEqual(nord.dark);
  });

  it('treats an unknown id as the app\'s own scheme', () => {
    expect(resolveScheme('nope', 'dark')).toEqual(xtermTheme('dark'));
  });

  it('tells dark backgrounds from light ones', () => {
    expect(isDarkBackground('#1e1e2e')).toBe(true);
    expect(isDarkBackground('#eff1f5')).toBe(false);
    expect(isDarkBackground('#002b36')).toBe(true);
    expect(isDarkBackground('#fdf6e3')).toBe(false);
  });
});
