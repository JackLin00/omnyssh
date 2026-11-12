// The remote shell's current folder, for SFTP uploads dropped on a terminal. SSH has
// no way to ask for it; a shell set up for it reports it after each command as
// OSC 7 (`ESC ] 7 ; file://host/path`), which zsh frameworks and fish often do
// already. `CWD_SETUP` makes a bash or zsh shell do it for the rest of its life.

/** An OSC 7 payload's host and folder, or null when it is not a `file://` URL. */
export function parseOsc7(data: string): { host: string; path: string } | null {
  const m = /^file:\/\/([^/]*)(\/.*)$/.exec(data);
  if (!m) return null;
  let path = m[2];
  try {
    path = decodeURIComponent(path);
  } catch {
    // A shell that does not encode the path; take it as it is.
  }
  return { host: m[1], path };
}

/** Follows OSC 7 reports. The first host named is the terminal's own; while another
 *  one reports (an `ssh` from this shell), the folder is unknown, since SFTP would
 *  look for it on the wrong machine. */
export function createCwdTracker(): { report: (data: string) => void; cwd: () => string | null } {
  let home: string | undefined;
  let cwd: string | null = null;
  return {
    report(data) {
      const r = parseOsc7(data);
      if (!r) return;
      if (r.host && home === undefined) home = r.host;
      cwd = !r.host || r.host === home ? r.path : null;
    },
    cwd: () => cwd
  };
}

/** Where an SFTP drop goes: the shell's folder, else the host's upload folder, else
 *  home. */
export function sftpFolder(cwd: string | null, uploadDir: string | null | undefined): string {
  return cwd ?? (uploadDir?.trim() || '~');
}

/** Typed into the shell to make it report its folder after every command, in bash
 *  and zsh, once per shell. The leading space keeps it out of the history
 *  (bash's ignorespace, zsh's HIST_IGNORE_SPACE). */
export const CWD_SETUP =
  ' if [ -z "$__OMNY_CWD" ]; then __OMNY_CWD=1; ' +
  `__omny_cwd() { printf '\\033]7;file://%s%s\\033\\\\' "\${HOSTNAME:-$HOST}" "$PWD"; }; ` +
  'if [ -n "$ZSH_VERSION" ]; then precmd_functions+=(__omny_cwd); ' +
  'else PROMPT_COMMAND="__omny_cwd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"; fi; fi\r';
