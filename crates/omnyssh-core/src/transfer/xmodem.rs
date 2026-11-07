//! XMODEM and YMODEM senders. They share blocks, CRCs and the ACK/NAK loop;
//! YMODEM adds a block 0 with the file's name and size before each file, and
//! an empty block 0 after the last one.
//!
//! The receiver picks the check: `C` asks for CRC-16, and then data goes in
//! 1024-byte blocks (128 for a short tail); NAK asks for the 8-bit checksum
//! and 128-byte blocks, as the original XMODEM.

use std::io::Read;
use std::time::{Duration, Instant};

use super::{Upload, UploadFile, UploadState, TRANSMIT_CHUNK};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_START: u8 = b'C';
/// Fills the last data block (CP/M's end of file).
const PAD: u8 = 0x1A;

/// How long the receiver gets to start: the user still has to type `loady`.
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one block waits for its answer before it is sent again.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);
/// Sends of one block after the first, before the transfer gives up.
const MAX_RETRIES: u32 = 10;
/// How long the line must stay quiet, after a NAK asks us to resend, before
/// we actually do: a burst of several then causes one resend, not one per
/// byte, and an ACK that was already on its way for an earlier duplicate
/// can't be mistaken for the next block's. A CRC `C` while we are waiting
/// does not start this (see the match arm below): only a NAK is the
/// receiver's own answer.
const SETTLE: Duration = Duration::from_millis(200);

/// What we send to cancel: 10 CANs, enough to get past the receiver's typical
/// 2-CAN debounce and any input buffering, then 10 backspaces to erase an
/// echoed command line. Matches lrzsz's `canit()` and gets past U-Boot's
/// xyzModem, whose `xyzModem_CAN_COUNT` is 3.
const CANCEL_SEQUENCE: [u8; 20] = [
    CAN, CAN, CAN, CAN, CAN, CAN, CAN, CAN, CAN, CAN, //
    0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    Xmodem,
    Ymodem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Waiting for `C`/NAK to begin the next file (YMODEM: its block 0, or
    /// the end block after the last file; XMODEM: block 1).
    Start,
    /// YMODEM block 0 is out.
    HeaderAck,
    /// YMODEM block 0 was ACKed; waiting for the `C` that asks for data.
    DataStart,
    /// A data block is out.
    BlockAck,
    /// EOT is out.
    EotAck,
    /// The empty YMODEM block 0 that ends the batch is out.
    EndAck,
    Final,
}

pub(crate) struct XmodemUpload {
    variant: Variant,
    files: Vec<UploadFile>,
    /// The file being sent; `files.len()` once all are.
    index: usize,
    /// CRC-16 and 1K blocks, or the checksum and 128-byte blocks.
    crc: bool,
    phase: Phase,
    /// The packet waiting for an answer, to send again.
    packet: Vec<u8>,
    /// Number of the next data block (wraps at 256; starts at 1).
    seq: u8,
    /// Data blocks of this file sent so far, for messages.
    blocks: u32,
    /// File bytes in `packet`.
    packet_data: u64,
    /// File bytes the receiver has ACKed.
    acked: u64,
    retries: u32,
    deadline: Instant,
    /// A resend is due at this time. Every byte received meanwhile pushes it
    /// back by `SETTLE` and is otherwise dropped (it is not an answer: it is
    /// noise, or a duplicate from before the line settles).
    resend_at: Option<Instant>,
    now: Instant,
    out: Vec<u8>,
    /// The last byte received was CAN; two in a row cancel.
    saw_can: bool,
    state: UploadState,
    leftover: Vec<u8>,
}

impl XmodemUpload {
    pub(crate) fn new(variant: Variant, files: Vec<UploadFile>, now: Instant) -> Self {
        let mut up = Self {
            variant,
            files,
            index: 0,
            crc: true,
            phase: Phase::Start,
            packet: Vec::new(),
            seq: 1,
            blocks: 0,
            packet_data: 0,
            acked: 0,
            retries: 0,
            deadline: now + START_TIMEOUT,
            resend_at: None,
            now,
            out: Vec::new(),
            saw_can: false,
            state: UploadState::Waiting,
            leftover: Vec::new(),
        };
        if variant == Variant::Ymodem {
            // Caught now, not when `block0_info` would otherwise be
            // silently truncated to fit the 1024-byte block (see `block`'s
            // `resize`).
            let too_long = up
                .files
                .iter()
                .find(|f| block0_info(f).len() > 1024)
                .map(|f| f.name.clone());
            if let Some(name) = too_long {
                up.fail(format!("{name} has a name too long for YMODEM"));
            }
        }
        up
    }

    fn on_byte(&mut self, b: u8) {
        if b == CAN {
            if self.saw_can {
                self.finish(UploadState::Failed(
                    "the device cancelled the transfer".into(),
                ));
                return;
            }
            self.saw_can = true;
            return;
        }
        self.saw_can = false;
        // The end block's ACK means the batch is done; accept it even while
        // a resend is pending, or we would resend into the shell and report
        // a finished upload as failed.
        if self.phase == Phase::EndAck && b == ACK {
            self.finish(UploadState::Done {
                sent_files: self.files.len(),
                skipped: 0,
            });
            return;
        }
        if self.resend_at.is_some() {
            self.resend_at = Some(self.now + SETTLE);
            return;
        }
        let start = b == CRC_START || b == NAK;
        match self.phase {
            Phase::Start if start => {
                if self.index == 0 {
                    self.crc = b == CRC_START;
                }
                match self.variant {
                    Variant::Xmodem => self.send_data_block(),
                    Variant::Ymodem if self.index < self.files.len() => self.send_header(),
                    Variant::Ymodem => {
                        let packet = self.block(0, &[], 128, 0);
                        self.send_packet(packet, Phase::EndAck);
                    }
                }
            }
            Phase::HeaderAck if b == ACK => {
                self.phase = Phase::DataStart;
                self.retries = 0;
                self.deadline = self.now + ACK_TIMEOUT;
            }
            Phase::DataStart if start => self.send_data_block(),
            Phase::BlockAck if b == ACK => {
                self.acked += self.packet_data;
                self.retries = 0;
                self.send_data_block();
            }
            Phase::EotAck if b == ACK => {
                self.index += 1;
                self.retries = 0;
                if self.variant == Variant::Xmodem {
                    self.finish(UploadState::Done {
                        sent_files: 1,
                        skipped: 0,
                    });
                } else {
                    self.phase = Phase::Start;
                    self.deadline = self.now + ACK_TIMEOUT;
                }
            }
            // Only a NAK starts a settle: it is itself the receiver's
            // answer. A stray `C` (U-Boot repeats it while waiting) is not
            // treated as a resend request: it could arrive before the
            // receiver's real answer to the packet, and on a slow link or a
            // long round trip the settle could expire first, sending a
            // duplicate whose ACK would be mistaken for the next block's.
            Phase::HeaderAck | Phase::BlockAck | Phase::EotAck | Phase::EndAck if b == NAK => {
                self.resend_at = Some(self.now + SETTLE);
            }
            // Anything else (a banner, line noise, an echo) is not an answer.
            _ => {}
        }
    }

    /// YMODEM block 0: `name NUL size [mtime]` (size decimal, mtime octal).
    fn send_header(&mut self) {
        let info = block0_info(&self.files[self.index]);
        let len = if info.len() > 128 { 1024 } else { 128 };
        let packet = self.block(0, &info, len, 0);
        self.start_file();
        self.send_packet(packet, Phase::HeaderAck);
    }

    /// The next data block of the current file, or EOT after the last.
    fn send_data_block(&mut self) {
        self.start_file();
        let size = self.files[self.index].size;
        let remaining = size - self.acked;
        if remaining == 0 {
            self.send_packet(vec![EOT], Phase::EotAck);
            return;
        }
        let len = if self.crc && remaining > 128 {
            1024
        } else {
            128
        };
        let want = remaining.min(len as u64) as usize;
        let mut data = vec![0u8; want];
        let name = self.files[self.index].name.clone();
        if let Err(e) = read_full(&mut self.files[self.index].reader, &mut data) {
            self.fail(format!("could not read {name}: {e}"));
            return;
        }
        let packet = self.block(self.seq, &data, len, PAD);
        self.seq = self.seq.wrapping_add(1);
        self.blocks += 1;
        self.packet_data = want as u64;
        self.send_packet(packet, Phase::BlockAck);
    }

    /// Resets the per-file counters when a file begins (called from `Start`).
    fn start_file(&mut self) {
        if self.phase == Phase::Start {
            self.seq = 1;
            self.blocks = 0;
            self.acked = 0;
            self.packet_data = 0;
            let file = &self.files[self.index];
            self.state = UploadState::Sending {
                file_index: self.index,
                file_count: self.files.len(),
                file_name: file.name.clone(),
                sent: 0,
                total: file.size,
            };
        }
    }

    fn block(&self, seq: u8, data: &[u8], len: usize, pad: u8) -> Vec<u8> {
        let mut p = Vec::with_capacity(len + 5);
        p.push(if len == 1024 { STX } else { SOH });
        p.push(seq);
        p.push(!seq);
        let start = p.len();
        p.extend_from_slice(data);
        p.resize(start + len, pad);
        if self.crc {
            let c = crc16(&p[start..]);
            p.extend_from_slice(&c.to_be_bytes());
        } else {
            let sum = p[start..].iter().fold(0u8, |s, &b| s.wrapping_add(b));
            p.push(sum);
        }
        p
    }

    fn send_packet(&mut self, packet: Vec<u8>, phase: Phase) {
        self.out.extend_from_slice(&packet);
        self.packet = packet;
        self.phase = phase;
        self.deadline = self.now + ACK_TIMEOUT;
    }

    fn resend(&mut self) {
        self.resend_at = None;
        self.retries += 1;
        if self.retries > MAX_RETRIES {
            let what = match self.phase {
                Phase::BlockAck => format!("block {}", self.blocks),
                Phase::EotAck => "end of file".to_string(),
                Phase::EndAck => "the end of the batch".to_string(),
                _ => "the file header".to_string(),
            };
            self.fail(format!("too many retries at {what}"));
            return;
        }
        let packet = self.packet.clone();
        self.out.extend_from_slice(&packet);
        self.deadline = self.now + ACK_TIMEOUT;
    }

    /// Our side gives up: tell the receiver too.
    fn fail(&mut self, message: String) {
        self.out.clear();
        self.out.extend_from_slice(&CANCEL_SEQUENCE);
        self.finish(UploadState::Failed(message));
    }

    fn finish(&mut self, state: UploadState) {
        self.phase = Phase::Final;
        self.resend_at = None;
        self.state = state;
    }
}

impl Upload for XmodemUpload {
    fn receive(&mut self, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() {
            if self.phase == Phase::Final {
                self.leftover.extend_from_slice(&data[i..]);
                return;
            }
            self.on_byte(b);
        }
    }

    fn transmit(&mut self, out: &mut Vec<u8>) {
        let n = self.out.len().min(TRANSMIT_CHUNK);
        out.extend(self.out.drain(..n));
    }

    fn tick(&mut self, now: Instant) {
        self.now = now;
        if let Some(t) = self.resend_at {
            if now >= t {
                self.resend();
                return;
            }
        }
        if now < self.deadline {
            return;
        }
        match self.phase {
            Phase::Start if self.index == 0 => self
                .fail("the receiver did not start (did you run loady/loadx on the device?)".into()),
            Phase::Start | Phase::DataStart => self.fail("the device stopped responding".into()),
            Phase::HeaderAck | Phase::BlockAck | Phase::EotAck | Phase::EndAck => self.resend(),
            Phase::Final => {}
        }
    }

    fn cancel(&mut self) {
        if self.phase == Phase::Final {
            return;
        }
        self.out.clear();
        self.out.extend_from_slice(&CANCEL_SEQUENCE);
        self.finish(UploadState::Cancelled);
    }

    fn state(&self) -> UploadState {
        match &self.state {
            UploadState::Sending {
                file_index,
                file_count,
                file_name,
                total,
                ..
            } => UploadState::Sending {
                file_index: *file_index,
                file_count: *file_count,
                file_name: file_name.clone(),
                sent: self.acked,
                total: *total,
            },
            s => s.clone(),
        }
    }

    fn leftover(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.leftover)
    }
}

/// YMODEM block 0's info: `name NUL size [SP mtime(octal)] NUL`. Must fit in
/// the 1024-byte block (see the caller in `new` that checks this).
fn block0_info(file: &UploadFile) -> Vec<u8> {
    let mut info = file.name.as_bytes().to_vec();
    info.push(0);
    info.extend_from_slice(file.size.to_string().as_bytes());
    if let Some(t) = file.modified {
        info.extend_from_slice(format!(" {t:o}").as_bytes());
    }
    info.push(0);
    info
}

/// Reads exactly `buf.len()` bytes; a file that ends early is an error.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<()> {
    r.read_exact(buf).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            std::io::Error::new(e.kind(), "the file got shorter while it was being sent")
        } else {
            e
        }
    })
}

/// CRC-16/XMODEM: polynomial 0x1021, initial value 0.
pub(crate) fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::test_util::{bytes, drain, file};

    /// A receiver for the tests: checks every packet and answers as lrzsz's
    /// `rb`/`rx` do.
    struct Receiver {
        ymodem: bool,
        crc: bool,
        buf: Vec<u8>,
        /// (name, size from block 0, data)
        files: Vec<(String, Option<u64>, Vec<u8>)>,
        /// Block 0s seen, raw.
        headers: Vec<Vec<u8>>,
        /// First byte of every data packet.
        kinds: Vec<u8>,
        expect: u8,
        eots: u32,
        in_file: bool,
        done: bool,
    }

    impl Receiver {
        fn new(ymodem: bool, crc: bool) -> Self {
            Self {
                ymodem,
                crc,
                buf: Vec::new(),
                files: Vec::new(),
                headers: Vec::new(),
                kinds: Vec::new(),
                expect: 1,
                eots: 0,
                in_file: !ymodem,
                done: false,
            }
        }

        fn start_byte(&self) -> u8 {
            if self.crc {
                CRC_START
            } else {
                NAK
            }
        }

        fn feed(&mut self, data: &[u8]) -> Vec<u8> {
            self.buf.extend_from_slice(data);
            let mut reply = Vec::new();
            loop {
                let Some(&first) = self.buf.first() else {
                    return reply;
                };
                if first == EOT {
                    self.buf.remove(0);
                    if self.ymodem && self.eots == 0 {
                        self.eots = 1;
                        reply.push(NAK);
                    } else {
                        self.eots = 0;
                        reply.push(ACK);
                        self.in_file = false;
                        if self.ymodem {
                            reply.push(CRC_START);
                        } else {
                            self.done = true;
                        }
                    }
                    continue;
                }
                let len = match first {
                    SOH => 128,
                    STX => 1024,
                    other => panic!("unexpected byte {other:#x}"),
                };
                let total = 3 + len + if self.crc { 2 } else { 1 };
                if self.buf.len() < total {
                    return reply;
                }
                let p: Vec<u8> = self.buf.drain(..total).collect();
                assert_eq!(p[1], !p[2], "block number complement");
                let data = &p[3..3 + len];
                if self.crc {
                    assert_eq!(crc16(data).to_be_bytes(), [p[3 + len], p[4 + len]]);
                } else {
                    let sum = data.iter().fold(0u8, |s, &b| s.wrapping_add(b));
                    assert_eq!(sum, p[3 + len]);
                }
                if self.ymodem && !self.in_file {
                    assert_eq!(p[1], 0, "block 0 between files");
                    self.headers.push(data.to_vec());
                    reply.push(ACK);
                    if data[0] == 0 {
                        self.done = true;
                        continue;
                    }
                    let nul = data.iter().position(|&b| b == 0).unwrap();
                    let name = String::from_utf8(data[..nul].to_vec()).unwrap();
                    let rest = &data[nul + 1..];
                    let end = rest.iter().position(|&b| b == b' ' || b == 0).unwrap();
                    let size = std::str::from_utf8(&rest[..end]).unwrap().parse().unwrap();
                    self.files.push((name, Some(size), Vec::new()));
                    self.in_file = true;
                    self.expect = 1;
                    reply.push(CRC_START);
                    continue;
                }
                if self.files.is_empty() {
                    self.files.push(("".into(), None, Vec::new()));
                }
                if p[1] == self.expect {
                    self.kinds.push(first);
                    self.files.last_mut().unwrap().2.extend_from_slice(data);
                    self.expect = self.expect.wrapping_add(1);
                } else {
                    assert_eq!(
                        p[1],
                        self.expect.wrapping_sub(1),
                        "only a repeat may be out of order"
                    );
                }
                reply.push(ACK);
            }
        }
    }

    fn t0() -> Instant {
        Instant::now()
    }

    /// Runs an upload against `rx` until neither side has anything to say.
    /// `start` is the `Instant` the upload was created with: when the upload
    /// has nothing to send, the clock is moved forward by `SETTLE` and ticked,
    /// to let a pending resend (the receiver's first YMODEM EOT NAK, say) go
    /// out.
    fn run(up: &mut dyn Upload, rx: &mut Receiver, start: Instant) {
        up.receive(&[rx.start_byte()]);
        let mut now = start;
        for _ in 0..100_000 {
            let mut out = drain(up);
            if out.is_empty() {
                now += SETTLE;
                up.tick(now);
                out = drain(up);
                if out.is_empty() {
                    return;
                }
            }
            let reply = rx.feed(&out);
            up.receive(&reply);
        }
        panic!("the upload did not finish");
    }

    #[test]
    fn crc16_check_value() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
    }

    #[test]
    fn xmodem_crc_sends_1k_blocks_and_pads_the_last() {
        let t = t0();
        let data = bytes(3000, 1);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t);
        let mut rx = Receiver::new(false, true);
        run(&mut up, &mut rx, t);
        assert!(rx.done);
        assert_eq!(rx.kinds, vec![STX, STX, STX]);
        let got = &rx.files[0].2;
        assert_eq!(got.len(), 3072);
        assert_eq!(&got[..3000], &data[..]);
        assert!(got[3000..].iter().all(|&b| b == PAD));
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_short_tail_goes_in_a_128_byte_block() {
        let t = t0();
        let data = bytes(1100, 2);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t);
        let mut rx = Receiver::new(false, true);
        run(&mut up, &mut rx, t);
        assert_eq!(rx.kinds, vec![STX, SOH]);
        assert_eq!(&rx.files[0].2[..1100], &data[..]);
    }

    #[test]
    fn nak_start_falls_back_to_checksum_and_128_byte_blocks() {
        let t = t0();
        let data = bytes(300, 3);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t);
        let mut rx = Receiver::new(false, false);
        run(&mut up, &mut rx, t);
        assert!(rx.done);
        assert_eq!(rx.kinds, vec![SOH, SOH, SOH]);
        assert_eq!(&rx.files[0].2[..300], &data[..]);
    }

    #[test]
    fn ymodem_sends_a_batch_with_names_sizes_and_an_end_block() {
        let t = t0();
        let a = bytes(5000, 4);
        let b = bytes(10, 5);
        let files = vec![
            file("a.bin", a.clone()),
            file("b.txt", b.clone()),
            file("empty", vec![]),
        ];
        let mut up = XmodemUpload::new(Variant::Ymodem, files, t);
        let mut rx = Receiver::new(true, true);
        run(&mut up, &mut rx, t);
        assert!(rx.done);
        let names: Vec<_> = rx.files.iter().map(|f| (f.0.as_str(), f.1)).collect();
        assert_eq!(
            names,
            vec![
                ("a.bin", Some(5000)),
                ("b.txt", Some(10)),
                ("empty", Some(0))
            ]
        );
        assert_eq!(&rx.files[0].2[..5000], &a[..]);
        assert_eq!(&rx.files[1].2[..10], &b[..]);
        assert!(rx.files[2].2.is_empty());
        // name NUL size SP mtime(octal) NUL, zero-filled
        let mut want = b"a.bin\x005000 ".to_vec();
        want.extend_from_slice(format!("{:o}", 1_700_000_000u64).as_bytes());
        want.push(0);
        assert_eq!(&rx.headers[0][..want.len()], &want[..]);
        assert!(rx.headers[0][want.len()..].iter().all(|&b| b == 0));
        assert!(rx.headers[3].iter().all(|&b| b == 0), "end block is empty");
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 3,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_long_name_gets_a_1k_block_0() {
        let name = "n".repeat(200);
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file(&name, vec![1])], t0());
        let mut out = Vec::new();
        up.receive(b"C");
        up.transmit(&mut out);
        assert_eq!(out[0], STX);
        assert_eq!(out.len(), 3 + 1024 + 2);
    }

    #[test]
    fn nak_resends_the_same_block_and_ack_moves_on() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 6))], t);
        up.receive(b"C");
        let first = drain(&mut up);
        up.receive(&[NAK]);
        assert!(drain(&mut up).is_empty());
        up.tick(t + SETTLE);
        assert_eq!(drain(&mut up), first);
        up.receive(&[ACK]);
        let second = drain(&mut up);
        assert_eq!(second[1], 2);
        match up.state() {
            UploadState::Sending { sent, total, .. } => assert_eq!((sent, total), (1024, 2048)),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn unanswered_blocks_are_resent_then_the_upload_gives_up() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(10, 7))], t);
        up.tick(t);
        up.receive(b"C");
        let block = drain(&mut up);
        let mut now = t;
        for _ in 0..MAX_RETRIES {
            now += ACK_TIMEOUT;
            up.tick(now);
            assert_eq!(drain(&mut up), block);
        }
        now += ACK_TIMEOUT;
        up.tick(now);
        assert_eq!(drain(&mut up), CANCEL_SEQUENCE.to_vec());
        assert_eq!(
            up.state(),
            UploadState::Failed("too many retries at block 1".into())
        );
    }

    #[test]
    fn a_receiver_that_never_starts_times_out() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file("a", vec![1])], t);
        up.tick(t + START_TIMEOUT - Duration::from_millis(1));
        assert_eq!(up.state(), UploadState::Waiting);
        up.tick(t + START_TIMEOUT);
        match up.state() {
            UploadState::Failed(m) => assert!(m.contains("loady"), "{m}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn two_cans_from_the_device_cancel_and_one_does_not() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(4096, 8))], t0());
        up.receive(b"C");
        drain(&mut up);
        up.receive(&[CAN, ACK]);
        assert!(!up.state().is_final());
        drain(&mut up);
        up.receive(&[CAN, CAN]);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device cancelled the transfer".into())
        );
        assert!(drain(&mut up).is_empty());
    }

    #[test]
    fn a_name_too_long_for_block_0_fails_clearly() {
        let name = "n".repeat(1100);
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file(&name, vec![1])], t0());
        assert_eq!(drain(&mut up), CANCEL_SEQUENCE.to_vec());
        assert_eq!(
            up.state(),
            UploadState::Failed(format!("{name} has a name too long for YMODEM"))
        );
    }

    #[test]
    fn cancel_sends_the_cancel_sequence_and_later_bytes_are_leftover() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(4096, 9))], t0());
        up.receive(b"C");
        drain(&mut up);
        up.cancel();
        assert_eq!(up.state(), UploadState::Cancelled);
        assert_eq!(drain(&mut up), CANCEL_SEQUENCE.to_vec());
        up.receive(b"=> ");
        assert_eq!(up.leftover(), b"=> ");
    }

    #[test]
    fn bytes_after_the_last_ack_are_leftover() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", vec![1, 2, 3])], t0());
        up.receive(b"C");
        drain(&mut up);
        up.receive(&[ACK]);
        assert_eq!(drain(&mut up), vec![EOT]);
        up.receive(b"\x06## Total Size = 0x3\r\n=> ");
        assert!(up.state().is_final());
        assert_eq!(up.leftover(), b"## Total Size = 0x3\r\n=> ");
    }

    #[test]
    fn noise_before_the_start_is_ignored() {
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", vec![1])], t0());
        up.receive(b"## Ready for binary (ymodem) download\r\n");
        assert!(drain(&mut up).is_empty());
        up.receive(b"C");
        assert_eq!(drain(&mut up)[0], SOH);
    }

    #[test]
    fn a_read_error_cancels_the_receiver() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("disk gone"))
            }
        }
        impl std::io::Seek for Broken {
            fn seek(&mut self, _: std::io::SeekFrom) -> std::io::Result<u64> {
                Ok(0)
            }
        }
        let f = UploadFile {
            name: "a".into(),
            size: 10,
            modified: None,
            reader: Box::new(Broken),
        };
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![f], t0());
        up.receive(b"C");
        assert_eq!(drain(&mut up), CANCEL_SEQUENCE.to_vec());
        assert_eq!(
            up.state(),
            UploadState::Failed("could not read a: disk gone".into())
        );
    }

    #[test]
    fn a_burst_of_naks_resends_once() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(100, 10))], t);
        up.receive(b"C");
        let block = drain(&mut up);
        up.receive(&[NAK, NAK]);
        assert!(drain(&mut up).is_empty());
        up.tick(t + SETTLE);
        assert_eq!(drain(&mut up), block);
        assert!(drain(&mut up).is_empty());
    }

    #[test]
    fn a_stray_c_while_waiting_is_ignored() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 11))], t);
        up.receive(b"C");
        drain(&mut up);
        up.receive(b"C");
        up.tick(t + SETTLE);
        assert!(drain(&mut up).is_empty());
        up.receive(&[ACK]);
        let second = drain(&mut up);
        assert_eq!(second[1], 2);
    }

    #[test]
    fn an_ack_during_the_settle_is_dropped() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 12))], t);
        up.receive(b"C");
        let block1 = drain(&mut up);
        up.receive(&[NAK]);
        up.tick(t + Duration::from_millis(100));
        up.receive(&[ACK]);
        assert!(drain(&mut up).is_empty());
        up.tick(t + Duration::from_millis(100) + SETTLE - Duration::from_millis(1));
        assert!(drain(&mut up).is_empty());
        up.tick(t + Duration::from_millis(100) + SETTLE);
        assert_eq!(drain(&mut up), block1);
    }

    #[test]
    fn bytes_during_the_settle_push_the_resend_back() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 13))], t);
        up.receive(b"C");
        let block1 = drain(&mut up);
        up.receive(&[NAK]);
        up.tick(t + Duration::from_millis(150));
        up.receive(&[0xFF]); // noise: not CAN/ACK/NAK/C, pushes the resend back
        up.tick(t + Duration::from_millis(250));
        assert!(drain(&mut up).is_empty());
        up.tick(t + Duration::from_millis(350));
        assert_eq!(drain(&mut up), block1);
    }

    #[test]
    fn block_numbers_wrap_after_255() {
        let t = t0();
        let data = bytes(300 * 1024, 14);
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a.bin", data.clone())], t);
        let mut rx = Receiver::new(false, true);
        run(&mut up, &mut rx, t);
        assert!(rx.done);
        assert_eq!(rx.files[0].2, data);
    }

    #[test]
    fn ymodem_in_checksum_mode() {
        let t = t0();
        let a = bytes(500, 15);
        let b = bytes(20, 16);
        let files = vec![file("a.bin", a.clone()), file("b.txt", b.clone())];
        let mut up = XmodemUpload::new(Variant::Ymodem, files, t);
        let mut rx = Receiver::new(true, false);
        run(&mut up, &mut rx, t);
        assert!(rx.done);
        let names: Vec<_> = rx.files.iter().map(|f| (f.0.as_str(), f.1)).collect();
        assert_eq!(names, vec![("a.bin", Some(500)), ("b.txt", Some(20))]);
        assert_eq!(&rx.files[0].2[..500], &a[..]);
        assert_eq!(&rx.files[1].2[..20], &b[..]);
    }

    #[test]
    fn header_retries_do_not_count_against_block_1() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file("a", bytes(10, 17))], t);
        up.receive(b"C");
        let header = drain(&mut up);
        let mut now = t;
        for _ in 0..8 {
            up.receive(&[NAK]);
            now += SETTLE;
            up.tick(now);
            assert_eq!(drain(&mut up), header);
        }
        up.receive(&[ACK]);
        up.receive(b"C");
        let block1 = drain(&mut up);
        for _ in 0..MAX_RETRIES {
            now += ACK_TIMEOUT;
            up.tick(now);
            assert_eq!(drain(&mut up), block1);
        }
        assert!(!up.state().is_final());
        now += ACK_TIMEOUT;
        up.tick(now);
        assert!(up.state().is_final());
    }

    #[test]
    fn chatter_during_the_settle_still_times_out() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Xmodem, vec![file("a", bytes(2048, 18))], t);
        up.receive(b"C");
        let block1 = drain(&mut up);
        up.receive(&[NAK]);
        let mut now = t;
        let mut resent = Vec::new();
        // The device keeps nagging with `C` (or sending line noise) the
        // whole time: the settle never finishes, but block 1's own deadline
        // must still fire.
        while now < t + ACK_TIMEOUT {
            now += Duration::from_millis(100);
            up.tick(now);
            resent.extend(drain(&mut up));
            up.receive(b"C");
        }
        assert_eq!(
            resent, block1,
            "block 1 should have been resent by its deadline"
        );
        // Keep up the chatter through MAX_RETRIES more deadlines: the retry
        // budget still runs out and the upload gives up.
        let mut tail = Vec::new();
        for _ in 0..MAX_RETRIES {
            let until = now + ACK_TIMEOUT;
            while now < until {
                now += Duration::from_millis(100);
                up.tick(now);
                tail.extend(drain(&mut up));
                up.receive(b"C");
            }
        }
        assert_eq!(
            up.state(),
            UploadState::Failed("too many retries at block 1".into())
        );
        assert!(tail.ends_with(&CANCEL_SEQUENCE));
    }

    #[test]
    fn the_end_ack_counts_during_a_settle() {
        let t = t0();
        let mut up = XmodemUpload::new(Variant::Ymodem, vec![file("a", vec![1, 2, 3])], t);
        up.receive(b"C");
        drain(&mut up); // block 0 (header)
        up.receive(&[ACK]);
        up.receive(b"C");
        drain(&mut up); // block 1
        up.receive(&[ACK]);
        drain(&mut up); // EOT
        up.receive(&[ACK]);
        up.receive(b"C");
        drain(&mut up); // the end block
        up.receive(&[NAK]);
        up.receive(&[ACK]);
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            }
        );
        up.tick(t + SETTLE);
        assert!(drain(&mut up).is_empty());
    }
}
