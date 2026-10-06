//! Plain-text session logs: what a terminal or a serial port printed, without the
//! control sequences that drew it on screen.
//!
//! Pure: raw bytes and the time they arrived go in, the text to append to the log
//! file comes out. Both loggers keep state between chunks, because a chunk boundary
//! can fall inside an escape sequence, a UTF-8 character or a hex row. The caller
//! owns the file.

use chrono::NaiveDateTime;

/// The line ending this platform's editors expect.
pub const NATIVE_NEWLINE: &str = if cfg!(windows) { "\r\n" } else { "\n" };

/// Bytes per row of a hex log, matching the serial tab's HEX display.
pub const HEX_ROW: usize = 16;

/// Where the stripper is inside the output stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Seq {
    /// Plain text.
    #[default]
    Ground,
    /// Right after ESC.
    Esc,
    /// ESC and one or more intermediates (`(`, `#`, …), waiting for the final byte.
    EscIntermediate,
    /// A CSI's parameters and intermediates, waiting for the final byte.
    Csi,
    /// An OSC's text, up to BEL or ST.
    Osc,
    /// A DCS, SOS, PM or APC string, up to ST.
    Str,
}

/// Turns a terminal's raw output into plain text lines, optionally stamped with
/// when each line started arriving.
///
/// - CSI, OSC, DCS/SOS/PM/APC strings and the other ESC sequences are dropped,
///   in their 7-bit and their C1 (U+0080–U+009F) forms;
/// - every C0 control except `\t` and `\n` is dropped, and so is DEL;
/// - `\n` (and so `\r\n`) ends a line; a lone `\r` is dropped, so what a progress
///   bar redraws in place is appended rather than overwritten;
/// - bytes that are not UTF-8 become U+FFFD.
#[derive(Debug)]
pub struct TextLogger {
    timestamps: bool,
    newline: &'static str,
    /// The start of a UTF-8 character whose remaining bytes are still to come.
    pending: Vec<u8>,
    seq: Seq,
    /// Nothing written on the current line yet.
    line_start: bool,
}

impl TextLogger {
    /// `newline` is what a line ends with in the file, normally [`NATIVE_NEWLINE`].
    pub fn new(timestamps: bool, newline: &'static str) -> Self {
        Self {
            timestamps,
            newline,
            pending: Vec::new(),
            seq: Seq::Ground,
            line_start: true,
        }
    }

    /// The text for one chunk of output, which arrived at `at` (local time). A line
    /// is stamped with the arrival of the chunk holding its first character; a blank
    /// line is not stamped. Call in arrival order.
    pub fn feed(&mut self, bytes: &[u8], at: NaiveDateTime) -> String {
        let text = self.decode(bytes);
        let mut out = String::with_capacity(text.len());
        for c in text.chars() {
            self.step(c, &mut out, at);
        }
        out
    }

    /// What closes the log: the end of a line left open. A character cut short by
    /// the stop is dropped.
    pub fn finish(&mut self) -> String {
        self.pending.clear();
        if self.line_start {
            String::new()
        } else {
            self.line_start = true;
            self.newline.to_string()
        }
    }

    /// Decode `bytes` after whatever an earlier chunk left unfinished, keeping the
    /// start of a character the chunk cuts off for the next one.
    fn decode(&mut self, bytes: &[u8]) -> String {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(bytes);
        let mut text = String::with_capacity(buf.len());
        let mut rest = &buf[..];
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    text.push_str(s);
                    break;
                }
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    // `valid_up_to` marks where the valid prefix ends.
                    text.push_str(std::str::from_utf8(valid).expect("valid prefix"));
                    match e.error_len() {
                        Some(len) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[len..];
                        }
                        // Cut off by the end of the chunk, not invalid (yet).
                        None => {
                            self.pending = after.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        text
    }

    fn step(&mut self, c: char, out: &mut String, at: NaiveDateTime) {
        // CAN and SUB abort any sequence; ESC starts a new one from anywhere,
        // which also makes `ESC \` (ST) end an OSC or a DCS.
        match c {
            '\u{18}' | '\u{1a}' => {
                self.seq = Seq::Ground;
                return;
            }
            '\u{1b}' => {
                self.seq = Seq::Esc;
                return;
            }
            _ => {}
        }
        match self.seq {
            Seq::Ground => self.ground(c, out, at),
            Seq::Esc => match c {
                '[' => self.seq = Seq::Csi,
                ']' => self.seq = Seq::Osc,
                'P' | 'X' | '^' | '_' => self.seq = Seq::Str,
                '\u{20}'..='\u{2f}' => self.seq = Seq::EscIntermediate,
                '\u{30}'..='\u{7e}' => self.seq = Seq::Ground,
                c => self.inside_sequence(c, out, at),
            },
            Seq::EscIntermediate => match c {
                '\u{20}'..='\u{2f}' => {}
                '\u{30}'..='\u{7e}' => self.seq = Seq::Ground,
                c => self.inside_sequence(c, out, at),
            },
            Seq::Csi => match c {
                '\u{20}'..='\u{3f}' => {}
                '\u{40}'..='\u{7e}' => self.seq = Seq::Ground,
                c => self.inside_sequence(c, out, at),
            },
            Seq::Osc => {
                if c == '\u{07}' || c == '\u{9c}' {
                    self.seq = Seq::Ground;
                }
            }
            Seq::Str => {
                if c == '\u{9c}' {
                    self.seq = Seq::Ground;
                }
            }
        }
    }

    /// A character that is no part of the escape or CSI sequence in progress. A C0
    /// control still acts, as a terminal executes it there; DEL is ignored; anything
    /// else breaks the sequence off and is handled as plain text.
    fn inside_sequence(&mut self, c: char, out: &mut String, at: NaiveDateTime) {
        match c {
            '\u{00}'..='\u{1f}' => self.control(c, out, at),
            '\u{7f}' => {}
            c => {
                self.seq = Seq::Ground;
                self.ground(c, out, at);
            }
        }
    }

    fn ground(&mut self, c: char, out: &mut String, at: NaiveDateTime) {
        match c {
            '\u{00}'..='\u{1f}' => self.control(c, out, at),
            '\u{7f}' => {}
            // The C1 forms of CSI, OSC and the DCS/SOS/PM/APC strings.
            '\u{9b}' => self.seq = Seq::Csi,
            '\u{9d}' => self.seq = Seq::Osc,
            '\u{90}' | '\u{98}' | '\u{9e}' | '\u{9f}' => self.seq = Seq::Str,
            // Every other C1 control.
            '\u{80}'..='\u{9f}' => {}
            c => self.print(c, out, at),
        }
    }

    fn control(&mut self, c: char, out: &mut String, at: NaiveDateTime) {
        match c {
            '\n' => {
                out.push_str(self.newline);
                self.line_start = true;
            }
            '\t' => self.print(c, out, at),
            _ => {}
        }
    }

    fn print(&mut self, c: char, out: &mut String, at: NaiveDateTime) {
        if self.line_start {
            if self.timestamps {
                out.push_str(&at.format("[%Y-%m-%d %H:%M:%S%.3f] ").to_string());
            }
            self.line_start = false;
        }
        out.push(c);
    }
}

/// Writes received bytes as a hex dump, [`HEX_ROW`] bytes per line (`55 AA 01 …`),
/// one continuous dump across chunks, like the serial tab's HEX display.
#[derive(Debug)]
pub struct HexLogger {
    newline: &'static str,
    /// Bytes on the current row.
    column: usize,
}

impl HexLogger {
    pub fn new(newline: &'static str) -> Self {
        Self { newline, column: 0 }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 3);
        for b in bytes {
            if self.column == HEX_ROW {
                out.push_str(self.newline);
                self.column = 0;
            }
            if self.column > 0 {
                out.push(' ');
            }
            out.push_str(&format!("{b:02X}"));
            self.column += 1;
        }
        out
    }

    /// The end of the last row.
    pub fn finish(&mut self) -> String {
        if self.column == 0 {
            String::new()
        } else {
            self.column = 0;
            self.newline.to_string()
        }
    }
}

/// One log's formatter: text (SSH, a serial tab showing text) or hex.
#[derive(Debug)]
pub enum SessionLogger {
    Text(TextLogger),
    Hex(HexLogger),
}

impl SessionLogger {
    pub fn feed(&mut self, bytes: &[u8], at: NaiveDateTime) -> String {
        match self {
            Self::Text(t) => t.feed(bytes, at),
            Self::Hex(h) => h.feed(bytes),
        }
    }

    pub fn finish(&mut self) -> String {
        match self {
            Self::Text(t) => t.finish(),
            Self::Hex(h) => h.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// A wall-clock time, as the GUI hands over `Local::now().naive_local()`: no
    /// time zone is involved, so the tests pass in any.
    fn at(h: u32, m: u32, s: u32, ms: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 6)
            .unwrap()
            .and_hms_milli_opt(h, m, s, ms)
            .unwrap()
    }

    /// Feed every chunk at the same time, without stamps, and finish.
    fn plain(chunks: &[&[u8]]) -> String {
        let mut log = TextLogger::new(false, "\n");
        let mut out: String = chunks.iter().map(|c| log.feed(c, at(0, 0, 0, 0))).collect();
        out.push_str(&log.finish());
        out
    }

    #[test]
    fn text_passes_and_both_line_ends_become_the_given_newline() {
        assert_eq!(plain(&[b"one\ntwo\r\nthree\n"]), "one\ntwo\nthree\n");
        let mut log = TextLogger::new(false, "\r\n");
        assert_eq!(log.feed(b"one\ntwo\r\n", at(0, 0, 0, 0)), "one\r\ntwo\r\n");
    }

    #[test]
    fn a_lone_carriage_return_is_dropped_and_the_rest_carries_on() {
        assert_eq!(plain(&[b"10%\r50%\r100%\r\n"]), "10%50%100%\n");
    }

    #[test]
    fn csi_sequences_are_stripped() {
        assert_eq!(
            plain(&[b"\x1b[1;31mred\x1b[0m \x1b[?2004hplain\x1b[2K\x1b[3 q\n"]),
            "red plain\n"
        );
    }

    #[test]
    fn osc_ends_at_bel_or_st() {
        assert_eq!(plain(&[b"\x1b]0;user@host: ~\x07$ ls\n"]), "$ ls\n");
        assert_eq!(
            plain(&[b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\\n"]),
            "link\n"
        );
    }

    #[test]
    fn dcs_sos_pm_and_apc_strings_end_at_st() {
        assert_eq!(
            plain(&[b"a\x1bP1$r0m\x1b\\b\x1bXsos\x1b\\c\x1b^pm\x1b\\d\x1b_apc\x1b\\e\n"]),
            "abcde\n"
        );
    }

    #[test]
    fn single_escape_sequences_are_stripped() {
        // Keypad modes, a charset with an intermediate, save/restore cursor, RIS.
        assert_eq!(plain(&[b"\x1b=\x1b>\x1b(Bok\x1b7\x1b8\x1bc\n"]), "ok\n");
    }

    #[test]
    fn c1_controls_are_stripped_like_their_escape_forms() {
        assert_eq!(
            plain(&["\u{9b}31mred\u{9b}0m\u{9d}0;t\u{9c}!\u{85}\n".as_bytes()]),
            "red!\n"
        );
    }

    #[test]
    fn controls_other_than_tab_and_newline_are_dropped() {
        assert_eq!(plain(&[b"a\tb\x07\x08c\x00d\x7f\x0e\n"]), "a\tbcd\n");
    }

    #[test]
    fn can_and_sub_abort_a_sequence() {
        assert_eq!(plain(&[b"\x1b[12\x18x\x1b]0;t\x1ay\n"]), "xy\n");
    }

    #[test]
    fn a_newline_inside_a_sequence_still_ends_the_line() {
        assert_eq!(plain(&[b"a\x1b[1\nm b\n"]), "a\n b\n");
    }

    #[test]
    fn a_non_ascii_character_breaks_a_sequence_off() {
        assert_eq!(plain(&["\x1b[1é\n".as_bytes()]), "é\n");
    }

    #[test]
    fn sequences_split_across_chunks_are_still_stripped() {
        assert_eq!(plain(&[b"\x1b", b"[3", b"1mred\x1b", b"[0m\n"]), "red\n");
        assert_eq!(plain(&[b"\x1b]0;ti", b"tle\x1b", b"\\$ \n"]), "$ \n");
    }

    #[test]
    fn utf8_split_across_chunks_is_decoded_whole() {
        let crab = "é🦀".as_bytes();
        assert_eq!(
            plain(&[&crab[..1], &crab[1..3], &crab[3..5], &crab[5..], b"\n"]),
            "é🦀\n"
        );
    }

    #[test]
    fn invalid_utf8_becomes_the_replacement_character() {
        assert_eq!(plain(&[b"a\xffb\xc3(\n"]), "a\u{fffd}b\u{fffd}(\n");
        // An invalid byte after a partial character carried over from the last chunk.
        assert_eq!(plain(&[b"x\xe2\x82", b"y\n"]), "x\u{fffd}y\n");
    }

    #[test]
    fn lines_are_stamped_with_the_arrival_of_their_first_character() {
        let mut log = TextLogger::new(true, "\n");
        let mut out = log.feed(b"one\n", at(14, 30, 5, 7));
        out += &log.feed(b"two\n\r\n\nthr", at(14, 30, 6, 120));
        out += &log.feed(b"ee\n\x1b[0m", at(14, 31, 0, 999));
        out += &log.finish();
        assert_eq!(
            out,
            "[2026-10-06 14:30:05.007] one\n\
             [2026-10-06 14:30:06.120] two\n\
             \n\
             \n\
             [2026-10-06 14:30:06.120] three\n"
        );
    }

    #[test]
    fn a_line_holding_only_sequences_or_returns_is_blank_and_unstamped() {
        let mut log = TextLogger::new(true, "\n");
        let out = log.feed(b"\x1b[2J\x1b[H\r\n\r\n\ttab\n", at(9, 0, 0, 0));
        assert_eq!(out, "\n\n[2026-10-06 09:00:00.000] \ttab\n");
    }

    #[test]
    fn finishing_ends_an_open_line_once() {
        let mut log = TextLogger::new(false, "\r\n");
        assert_eq!(log.feed(b"$ ", at(0, 0, 0, 0)), "$ ");
        assert_eq!(log.finish(), "\r\n");
        assert_eq!(log.finish(), "");
        // A character cut short by the stop is dropped, not written as U+FFFD.
        let mut log = TextLogger::new(false, "\n");
        assert_eq!(log.feed(b"\xe2\x82", at(0, 0, 0, 0)), "");
        assert_eq!(log.finish(), "");
    }

    #[test]
    fn hex_rows_hold_sixteen_bytes_across_chunks() {
        let mut log = HexLogger::new("\n");
        let bytes: Vec<u8> = (0u8..18).collect();
        let mut out = log.feed(&bytes[..5]);
        out += &log.feed(&bytes[5..]);
        assert_eq!(
            out,
            "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F\n10 11"
        );
        assert_eq!(log.finish(), "\n");
        assert_eq!(log.finish(), "");
    }

    #[test]
    fn hex_uses_the_given_newline_and_upper_case() {
        let mut log = HexLogger::new("\r\n");
        let mut out = log.feed(&[0x55, 0xaa]);
        out += &log.feed(&[0xff; 14]);
        out += &log.feed(&[0x01]);
        out += &log.finish();
        assert_eq!(
            out,
            "55 AA FF FF FF FF FF FF FF FF FF FF FF FF FF FF\r\n01\r\n"
        );
    }

    #[test]
    fn a_hex_log_is_never_stamped() {
        let mut log = SessionLogger::Hex(HexLogger::new("\n"));
        assert_eq!(log.feed(b"\n", at(1, 2, 3, 4)), "0A");
        assert_eq!(log.finish(), "\n");
    }
}
