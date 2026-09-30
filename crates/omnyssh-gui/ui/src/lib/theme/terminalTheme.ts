// xterm renders outside the DOM/CSS-variable system, so its colours can't be CSS
// tokens — they're derived here from the brandbook's ink/paper language (§5.1) and
// pushed to every live terminal on theme toggle. Background/foreground/cursor mirror
// the `--surface`/`--text`/`--accent` tokens; the 16 ANSI colours are the terminal's
// content palette (what remote programs emit), reusing the app's server-state colours
// for red/green/yellow and cool blue/violet accents to stay on the monochrome brand
// tone. Both maps are tuned for AA-legible reading on their surface.
import type { ITheme } from '@xterm/xterm';
import type { ISearchDecorationOptions } from '@xterm/addon-search';
import type { Theme } from '$lib/stores/theme';

const dark: ITheme = {
  background: '#212121', // --surface (dark)
  foreground: '#e4e7ec', // paper, softened for sustained reading
  cursor: '#ffffff', // --accent (dark)
  cursorAccent: '#212121',
  selectionBackground: 'rgba(255, 255, 255, 0.18)',
  black: '#3a3f45',
  red: '#f07777', // --status-crit (dark)
  green: '#5fd08a', // --status-ok (dark)
  yellow: '#e8c15a', // --status-warn (dark)
  blue: '#7aa2f7',
  magenta: '#bb9af7',
  cyan: '#7dcfff',
  white: '#c8ccd4',
  brightBlack: '#5b6066', // graphite
  brightRed: '#ff8f8f',
  brightGreen: '#7fe0a3',
  brightYellow: '#f0d178',
  brightBlue: '#9ab8ff',
  brightMagenta: '#d0b8ff',
  brightCyan: '#9ee0ff',
  brightWhite: '#ffffff'
};

const light: ITheme = {
  background: '#ffffff', // --surface (light)
  foreground: '#212121', // ink
  cursor: '#212121', // --accent (light)
  cursorAccent: '#ffffff',
  selectionBackground: 'rgba(33, 33, 33, 0.15)',
  black: '#212121',
  red: '#c0392b', // --status-crit (light)
  green: '#1e824c', // --status-ok (light)
  yellow: '#b7791f', // --status-warn (light)
  blue: '#2f6feb',
  magenta: '#8250df',
  cyan: '#1b7c83',
  white: '#aeb7c2', // mist
  brightBlack: '#5b6066', // graphite
  brightRed: '#d64545',
  brightGreen: '#2a9d5c',
  brightYellow: '#c88a2a',
  brightBlue: '#4785ff',
  brightMagenta: '#9a5cf0',
  brightCyan: '#2596a0',
  brightWhite: '#212121'
};

const THEMES: Record<Theme, ITheme> = { dark, light };

/** The xterm theme object for the app theme; applied to every open terminal on toggle. */
export function xtermTheme(theme: Theme): ITheme {
  return THEMES[theme];
}

// Search highlights, for the same reason as the palette above: xterm draws them, not
// CSS. Every match gets a quiet wash; the current one takes the warning yellow so it
// stands out on either surface. The addon requires #RRGGBB. The overview-ruler colours
// only show once a terminal turns `overviewRulerWidth` on; none does today, so they are
// unused decoration until one does.
const SEARCH: Record<Theme, ISearchDecorationOptions> = {
  dark: {
    matchBackground: '#4a5058',
    matchOverviewRuler: '#7a8088',
    activeMatchBackground: '#6e5818',
    activeMatchColorOverviewRuler: '#e8c15a'
  },
  light: {
    matchBackground: '#dde2e8',
    matchOverviewRuler: '#aeb7c2',
    activeMatchBackground: '#f0d178',
    activeMatchColorOverviewRuler: '#b7791f'
  }
};

export function searchDecorations(t: Theme): ISearchDecorationOptions {
  return SEARCH[t];
}
