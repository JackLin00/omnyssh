// The terminal colour schemes a user can pick in Settings → Terminal colors. Each comes
// as a dark and, where the scheme has one, a light variant; the app theme decides which
// is shown. Values are the schemes' published Windows Terminal palettes (sources below);
// `omnyssh` is the app's own palette from terminalTheme.ts.
import type { ITheme } from '@xterm/xterm';
import type { Theme } from '$lib/stores/theme';
import { xtermTheme } from './terminalTheme';

/** A full palette: the four UI colours and the sixteen ANSI ones, all #rrggbb. */
export type SchemeColors = Required<
  Pick<
    ITheme,
    | 'background' | 'foreground' | 'cursor' | 'selectionBackground'
    | 'black' | 'red' | 'green' | 'yellow' | 'blue' | 'magenta' | 'cyan' | 'white'
    | 'brightBlack' | 'brightRed' | 'brightGreen' | 'brightYellow'
    | 'brightBlue' | 'brightMagenta' | 'brightCyan' | 'brightWhite'
  >
>;

export interface TerminalScheme {
  id: string;
  name: string;
  /** Variant names shown on the picker, e.g. Mocha / Latte. */
  darkName: string;
  lightName: string | null;
  dark: SchemeColors | null; // null only for `omnyssh`, resolved from terminalTheme
  light: SchemeColors | null;
}

// Sources (all from iTerm2-Color-Schemes, github.com/mbadolato/iTerm2-Color-Schemes,
// windowsterminal/*.json — the published Windows Terminal palette for each scheme):
// - catppuccin: Catppuccin Mocha.json, Catppuccin Latte.json
// - tokyonight: TokyoNight Night.json, TokyoNight Day.json
// - onehalf: One Half Dark.json, One Half Light.json
// - github: GitHub Dark Default.json, GitHub Light Default.json -- the "Default" pair is
//   GitHub's current Primer palette (what github.com and its terminal themes ship today);
//   the repo has no plain "GitHub Light.json" and the plain "GitHub Dark.json" there is an
//   older, less saturated palette, not the one this scheme is meant to preview
// - gruvbox: Gruvbox Dark.json, Gruvbox Light.json
// - solarized: iTerm2 Solarized Dark.json, iTerm2 Solarized Light.json (the repo has no
//   plain "Solarized Dark/Light.json"; these "iTerm2 Solarized" files carry Ethan
//   Schoonover's original Solarized values)
// - rosepine: Rose Pine.json, Rose Pine Dawn.json
// - nord: Nord.json for background/foreground/the normal ANSI eight/most brights, but
//   cursor, selectionBackground and brightBlack come from nordtheme/alacritty's own port
//   instead (github.com/nordtheme/alacritty, src/nord.yaml, branch main): Nord.json's
//   values for those three (#eceff4, #eceff4, #596377) are not Nord's own canonical nord4/
//   nord3 tones, so the project's own port is the more authoritative source for them
// - dracula: Dracula.json (dark only, as published)
// Every file above already had cursorColor and selectionBackground, so no fallback
// substitution was needed.
export const TERMINAL_SCHEMES: readonly TerminalScheme[] = [
  { id: 'omnyssh', name: 'Follow the app', darkName: 'Dark', lightName: 'Light', dark: null, light: null },
  {
    id: 'catppuccin',
    name: 'Catppuccin',
    darkName: 'Mocha',
    lightName: 'Latte',
    dark: {
      background: '#1e1e2e', foreground: '#cdd6f4', cursor: '#f5e0dc', selectionBackground: '#f5e0dc',
      black: '#45475a', red: '#f38ba8', green: '#a6e3a1', yellow: '#f9e2af',
      blue: '#89b4fa', magenta: '#f5c2e7', cyan: '#94e2d5', white: '#bac2de',
      brightBlack: '#585b70', brightRed: '#f7aec2', brightGreen: '#c2ecbf', brightYellow: '#fcd682',
      brightBlue: '#aeccfc', brightMagenta: '#f398da', brightCyan: '#b1eae1', brightWhite: '#a6adc8'
    },
    light: {
      background: '#eff1f5', foreground: '#4c4f69', cursor: '#dc8a78', selectionBackground: '#dc8a78',
      black: '#bcc0cc', red: '#d20f39', green: '#40a02b', yellow: '#df8e1d',
      blue: '#1e66f5', magenta: '#ea76cb', cyan: '#179299', white: '#5c5f77',
      brightBlack: '#acb0be', brightRed: '#e7103f', brightGreen: '#46b02f', brightYellow: '#e49931',
      brightBlue: '#3878f6', brightMagenta: '#ef95d7', brightCyan: '#19a1a8', brightWhite: '#6c6f85'
    }
  },
  {
    id: 'tokyonight',
    name: 'Tokyo Night',
    darkName: 'Night',
    lightName: 'Day',
    dark: {
      background: '#1a1b26', foreground: '#c0caf5', cursor: '#c0caf5', selectionBackground: '#283457',
      black: '#15161e', red: '#f7768e', green: '#9ece6a', yellow: '#e0af68',
      blue: '#7aa2f7', magenta: '#bb9af7', cyan: '#7dcfff', white: '#a9b1d6',
      brightBlack: '#414868', brightRed: '#f7768e', brightGreen: '#9ece6a', brightYellow: '#e0af68',
      brightBlue: '#7aa2f7', brightMagenta: '#bb9af7', brightCyan: '#7dcfff', brightWhite: '#c0caf5'
    },
    light: {
      background: '#e1e2e7', foreground: '#3760bf', cursor: '#3760bf', selectionBackground: '#99a7df',
      black: '#e9e9ed', red: '#f52a65', green: '#587539', yellow: '#8c6c3e',
      blue: '#2e7de9', magenta: '#9854f1', cyan: '#007197', white: '#6172b0',
      brightBlack: '#a1a6c5', brightRed: '#f52a65', brightGreen: '#587539', brightYellow: '#8c6c3e',
      brightBlue: '#2e7de9', brightMagenta: '#9854f1', brightCyan: '#007197', brightWhite: '#3760bf'
    }
  },
  {
    id: 'onehalf',
    name: 'One Half',
    darkName: 'Dark',
    lightName: 'Light',
    dark: {
      background: '#282c34', foreground: '#dcdfe4', cursor: '#a3b3cc', selectionBackground: '#474e5d',
      black: '#282c34', red: '#e06c75', green: '#98c379', yellow: '#e5c07b',
      blue: '#61afef', magenta: '#c678dd', cyan: '#56b6c2', white: '#dcdfe4',
      brightBlack: '#5d677a', brightRed: '#e06c75', brightGreen: '#98c379', brightYellow: '#e5c07b',
      brightBlue: '#61afef', brightMagenta: '#c678dd', brightCyan: '#56b6c2', brightWhite: '#dcdfe4'
    },
    light: {
      background: '#fafafa', foreground: '#383a42', cursor: '#a5b4e5', selectionBackground: '#bfceff',
      black: '#383a42', red: '#e45649', green: '#50a14f', yellow: '#c18401',
      blue: '#0184bc', magenta: '#a626a4', cyan: '#0997b3', white: '#bababa',
      brightBlack: '#4f525e', brightRed: '#e06c75', brightGreen: '#98c379', brightYellow: '#d8b36e',
      brightBlue: '#61afef', brightMagenta: '#c678dd', brightCyan: '#56b6c2', brightWhite: '#ffffff'
    }
  },
  {
    id: 'github',
    name: 'GitHub',
    darkName: 'Dark',
    lightName: 'Light',
    dark: {
      background: '#0d1117', foreground: '#e6edf3', cursor: '#2f81f7', selectionBackground: '#e6edf3',
      black: '#484f58', red: '#ff7b72', green: '#3fb950', yellow: '#d29922',
      blue: '#58a6ff', magenta: '#bc8cff', cyan: '#39c5cf', white: '#b1bac4',
      brightBlack: '#6e7681', brightRed: '#ffa198', brightGreen: '#56d364', brightYellow: '#e3b341',
      brightBlue: '#79c0ff', brightMagenta: '#d2a8ff', brightCyan: '#56d4dd', brightWhite: '#ffffff'
    },
    light: {
      background: '#ffffff', foreground: '#1f2328', cursor: '#0969da', selectionBackground: '#1f2328',
      black: '#24292f', red: '#cf222e', green: '#116329', yellow: '#4d2d00',
      blue: '#0969da', magenta: '#8250df', cyan: '#1b7c83', white: '#6e7781',
      brightBlack: '#57606a', brightRed: '#a40e26', brightGreen: '#1a7f37', brightYellow: '#633c01',
      brightBlue: '#218bff', brightMagenta: '#a475f9', brightCyan: '#3192aa', brightWhite: '#8c959f'
    }
  },
  {
    id: 'gruvbox',
    name: 'Gruvbox',
    darkName: 'Dark',
    lightName: 'Light',
    dark: {
      background: '#282828', foreground: '#ebdbb2', cursor: '#ebdbb2', selectionBackground: '#665c54',
      black: '#282828', red: '#cc241d', green: '#98971a', yellow: '#d79921',
      blue: '#458588', magenta: '#b16286', cyan: '#689d6a', white: '#a89984',
      brightBlack: '#928374', brightRed: '#fb4934', brightGreen: '#b8bb26', brightYellow: '#fabd2f',
      brightBlue: '#83a598', brightMagenta: '#d3869b', brightCyan: '#8ec07c', brightWhite: '#ebdbb2'
    },
    light: {
      background: '#fbf1c7', foreground: '#3c3836', cursor: '#3c3836', selectionBackground: '#3c3836',
      black: '#fbf1c7', red: '#cc241d', green: '#98971a', yellow: '#d79921',
      blue: '#458588', magenta: '#b16286', cyan: '#689d6a', white: '#7c6f64',
      brightBlack: '#928374', brightRed: '#9d0006', brightGreen: '#79740e', brightYellow: '#b57614',
      brightBlue: '#076678', brightMagenta: '#8f3f71', brightCyan: '#427b58', brightWhite: '#3c3836'
    }
  },
  {
    id: 'solarized',
    name: 'Solarized',
    darkName: 'Dark',
    lightName: 'Light',
    dark: {
      background: '#002b36', foreground: '#839496', cursor: '#839496', selectionBackground: '#073642',
      black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900',
      blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#eee8d5',
      brightBlack: '#335e69', brightRed: '#cb4b16', brightGreen: '#586e75', brightYellow: '#657b83',
      brightBlue: '#839496', brightMagenta: '#6c71c4', brightCyan: '#93a1a1', brightWhite: '#fdf6e3'
    },
    light: {
      background: '#fdf6e3', foreground: '#657b83', cursor: '#657b83', selectionBackground: '#eee8d5',
      black: '#073642', red: '#dc322f', green: '#859900', yellow: '#b58900',
      blue: '#268bd2', magenta: '#d33682', cyan: '#2aa198', white: '#bbb5a2',
      brightBlack: '#002b36', brightRed: '#cb4b16', brightGreen: '#586e75', brightYellow: '#657b83',
      brightBlue: '#839496', brightMagenta: '#6c71c4', brightCyan: '#93a1a1', brightWhite: '#fdf6e3'
    }
  },
  {
    id: 'rosepine',
    name: 'Rosé Pine',
    darkName: 'Main',
    lightName: 'Dawn',
    dark: {
      background: '#191724', foreground: '#e0def4', cursor: '#e0def4', selectionBackground: '#403d52',
      black: '#26233a', red: '#eb6f92', green: '#31748f', yellow: '#f6c177',
      blue: '#9ccfd8', magenta: '#c4a7e7', cyan: '#ebbcba', white: '#e0def4',
      brightBlack: '#6e6a86', brightRed: '#eb6f92', brightGreen: '#31748f', brightYellow: '#f6c177',
      brightBlue: '#9ccfd8', brightMagenta: '#c4a7e7', brightCyan: '#ebbcba', brightWhite: '#e0def4'
    },
    light: {
      background: '#faf4ed', foreground: '#575279', cursor: '#575279', selectionBackground: '#dfdad9',
      black: '#f2e9e1', red: '#b4637a', green: '#286983', yellow: '#ea9d34',
      blue: '#56949f', magenta: '#907aa9', cyan: '#d7827e', white: '#575279',
      brightBlack: '#9893a5', brightRed: '#b4637a', brightGreen: '#286983', brightYellow: '#ea9d34',
      brightBlue: '#56949f', brightMagenta: '#907aa9', brightCyan: '#d7827e', brightWhite: '#575279'
    }
  },
  {
    id: 'nord',
    name: 'Nord',
    darkName: 'Nord',
    lightName: null,
    dark: {
      background: '#2e3440', foreground: '#d8dee9', cursor: '#d8dee9', selectionBackground: '#4c566a',
      black: '#3b4252', red: '#bf616a', green: '#a3be8c', yellow: '#ebcb8b',
      blue: '#81a1c1', magenta: '#b48ead', cyan: '#88c0d0', white: '#e5e9f0',
      brightBlack: '#4c566a', brightRed: '#bf616a', brightGreen: '#a3be8c', brightYellow: '#ebcb8b',
      brightBlue: '#81a1c1', brightMagenta: '#b48ead', brightCyan: '#8fbcbb', brightWhite: '#eceff4'
    },
    light: null
  },
  {
    id: 'dracula',
    name: 'Dracula',
    darkName: 'Dracula',
    lightName: null,
    dark: {
      background: '#282a36', foreground: '#f8f8f2', cursor: '#f8f8f2', selectionBackground: '#44475a',
      black: '#21222c', red: '#ff5555', green: '#50fa7b', yellow: '#f1fa8c',
      blue: '#bd93f9', magenta: '#ff79c6', cyan: '#8be9fd', white: '#f8f8f2',
      brightBlack: '#6272a4', brightRed: '#ff6e6e', brightGreen: '#69ff94', brightYellow: '#ffffa5',
      brightBlue: '#d6acff', brightMagenta: '#ff92df', brightCyan: '#a4ffff', brightWhite: '#ffffff'
    },
    light: null
  }
];

export function resolveScheme(id: string, app: Theme): ITheme {
  const s = TERMINAL_SCHEMES.find((x) => x.id === id);
  if (!s || s.id === 'omnyssh' || !s.dark) return xtermTheme(app);
  return app === 'light' && s.light ? s.light : s.dark;
}

/** Whether a #rrggbb background is dark (relative luminance below the midpoint). */
export function isDarkBackground(hex: string): boolean {
  const n = parseInt(hex.slice(1, 7), 16);
  const channel = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  const l = 0.2126 * channel((n >> 16) & 255) + 0.7152 * channel((n >> 8) & 255) + 0.0722 * channel(n & 255);
  return l < 0.18;
}
