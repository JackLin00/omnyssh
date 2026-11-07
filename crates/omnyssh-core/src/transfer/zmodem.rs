//! ZMODEM sender for `rz`, wrapping `zmodem2`'s sans-IO `Sender`.
//!
//! What this adds on top: reading the files, the timers (`zmodem2` has none),
//! telling a skipped file from a sent one, and the [`Upload`] states.

use std::io::{Read, SeekFrom};
use std::time::{Duration, Instant};

use zmodem2::{Action, Error, Event, FileInfo, Position, Sender};

use super::{Upload, UploadFile, UploadState, TRANSMIT_CHUNK};

/// How long `rz` gets to answer with ZRINIT after it was started.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// Frame type 0 (ZRQINIT), in `zmodem2`'s hex header encoding. No ZRQINIT is
/// ever sent (see `pump`): lrzsz's `rz` announces itself with ZRINIT
/// unprompted on start, and keeps repeating it, so asking is unnecessary;
/// anything sent before `rz` is actually running instead reaches the shell,
/// whose cooked-mode echo of our own ZRQINIT would otherwise bounce forever
/// between that echo and `zmodem2`'s "answer any header while waiting"
/// fallback, which itself replies with another ZRQINIT.
const ZRQINIT_PREFIX: &[u8] = b"**\x18B00";
/// Once rz has answered, the upload fails when nothing was handed to the
/// caller to send and no file data was read or completed for this long.
/// `zmodem2`'s `Sender` never resends on its own, and it ignores a repeated
/// ZRINIT/ZRPOS/ZFERR once past the handshake, so rz's own header retries
/// (every ~10 s) are not evidence of progress and must not reset this. At
/// 2400 baud an 8 KiB chunk drains in ~34 s, inside the limit.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
/// A regular file readable by all, writable by its owner: without a mode `rz`
/// creates the file `0600`.
const FILE_MODE: u32 = 0o100_644;
/// zmodem2's file name buffer; a longer one makes `start_file` fail with an
/// opaque "out of memory" instead of a clear message.
const MAX_NAME: usize = 256;
/// ZDLE doubles as CAN. `rz` cancels by sending a run of them; five in a row
/// never occur otherwise (lrzsz's own threshold).
const CAN: u8 = 0x18;
const CANCEL_RUN: u32 = 5;

pub(crate) struct ZmodemUpload {
    sender: Sender,
    files: Vec<UploadFile>,
    index: usize,
    /// `start_file` for `files[index]` is still to be accepted (it refuses
    /// while bytes are queued).
    need_start: bool,
    need_finish: bool,
    /// The current file had data read: a file completed without any was
    /// skipped by the receiver (ZSKIP; `zmodem2` reports it as completed).
    read_any: bool,
    sent: u64,
    skipped: usize,
    /// The receiver has answered (ZRINIT seen, as far as we can tell).
    answered: bool,
    /// `submit_wire` consumed input since the last `WriteWire`: while
    /// `!answered`, the next `WriteWire` that is not a (suppressed) ZRQINIT
    /// is therefore the ZFILE queued in response to ZRINIT, the only other
    /// bytes the sender produces from input before it has answered.
    after_input: bool,
    started: Instant,
    /// Set at each progress event; `tick` bumps `last_progress` to its own
    /// `now` and clears this the next time it runs, rather than stamping
    /// `now` at the event itself, which would be the *previous* tick's time:
    /// a long gap between ticks (the host was asleep) must not read as a
    /// stall the moment ticking resumes.
    progressed: bool,
    last_progress: Instant,
    now: Instant,
    inbuf: Vec<u8>,
    /// CANs at the end of what the receiver sent so far.
    cans: u32,
    out: Vec<u8>,
    /// What to report once `abort()`'s cancel sequence is out; `None` means
    /// the receiver cancelled.
    abort_state: Option<UploadState>,
    state: UploadState,
    leftover: Vec<u8>,
}

impl ZmodemUpload {
    pub(crate) fn new(files: Vec<UploadFile>, now: Instant) -> Self {
        let mut up = Self {
            // Only fails when its own buffer cannot hold ZRQINIT.
            sender: Sender::new().expect("ZRQINIT fits"),
            files,
            index: 0,
            need_start: true,
            need_finish: false,
            read_any: false,
            sent: 0,
            skipped: 0,
            answered: false,
            after_input: false,
            started: now,
            progressed: false,
            last_progress: now,
            now,
            inbuf: Vec::new(),
            cans: 0,
            out: Vec::new(),
            abort_state: None,
            state: UploadState::Waiting,
            leftover: Vec::new(),
        };
        if let Some(f) = up.files.iter().find(|f| f.size > u64::from(u32::MAX)) {
            let msg = format!("{} is larger than ZMODEM's 4 GiB limit", f.name);
            up.fail(UploadState::Failed(msg));
        } else if let Some(f) = up.files.iter().find(|f| f.name.len() > MAX_NAME) {
            let msg = format!("{} has a name longer than ZMODEM's 256 bytes", f.name);
            up.fail(UploadState::Failed(msg));
        }
        up.pump();
        up
    }

    /// Runs the sender until it waits for input or `out` holds a chunk.
    fn pump(&mut self) {
        while self.out.len() < TRANSMIT_CHUNK {
            if !self.state.is_final() {
                self.queue_next();
            }
            match self.sender.poll() {
                Action::WriteWire(bytes) => {
                    let n = bytes.len();
                    // No ZRQINIT ever reaches the wire (see `ZRQINIT_PREFIX`);
                    // this also drops the one `Sender::new()` queues itself.
                    let is_zrqinit = !self.answered && bytes.starts_with(ZRQINIT_PREFIX);
                    if !is_zrqinit {
                        self.out.extend_from_slice(bytes);
                    }
                    self.sender.wire_written(n);
                    if !self.answered && self.after_input && !is_zrqinit {
                        self.answered = true;
                        self.progressed = true;
                    }
                    self.after_input = false;
                }
                Action::ReadFile { offset, max_len } => self.read_file(offset, max_len),
                Action::Event(Event::FileCompleted) => {
                    self.answered = true;
                    self.progressed = true;
                    if !self.read_any && self.files[self.index].size > 0 {
                        self.skipped += 1;
                    }
                    self.index += 1;
                    self.read_any = false;
                    if self.index < self.files.len() {
                        self.need_start = true;
                    } else {
                        self.need_finish = true;
                    }
                }
                Action::Event(Event::SessionCompleted) => {
                    self.progressed = true;
                    self.state = UploadState::Done {
                        sent_files: self.files.len() - self.skipped,
                        skipped: self.skipped,
                    };
                    self.leftover.append(&mut self.inbuf);
                }
                Action::Event(Event::Aborted) => {
                    self.state = self.abort_state.take().unwrap_or_else(|| {
                        UploadState::Failed("the device cancelled the transfer".into())
                    });
                    // Unread protocol bytes: noise in a terminal.
                    self.inbuf.clear();
                }
                Action::Event(_) => {}
                _ if self.state.is_final() || self.inbuf.is_empty() => return,
                _ => match self.sender.submit_wire(&self.inbuf) {
                    Ok(0) => return,
                    Ok(n) => {
                        self.inbuf.drain(..n);
                        self.after_input = true;
                    }
                    // A corrupt header: the sender has queued a ZNAK and
                    // the receiver sends it again.
                    Err(_) => self.inbuf.clear(),
                },
            }
        }
    }

    /// Offers the next file or ends the session when one is due; the sender
    /// refuses both while bytes are queued, so it is retried each round.
    fn queue_next(&mut self) {
        let result = if self.need_start {
            let file = &self.files[self.index];
            let mut info =
                FileInfo::new(file.name.as_bytes(), Some(Position::new(file.size as u32)))
                    .with_mode(FILE_MODE);
            if let Some(t) = file.modified.and_then(|t| u32::try_from(t).ok()) {
                info = info.with_modified(t);
            }
            self.sender
                .start_file(info)
                .map(|()| self.need_start = false)
        } else if self.need_finish {
            self.sender.finish().map(|()| self.need_finish = false)
        } else {
            Ok(())
        };
        match result {
            Ok(()) | Err(Error::Backpressure) => {}
            Err(e) => self.fail(UploadState::Failed(format!("ZMODEM: {e}"))),
        }
    }

    fn read_file(&mut self, offset: Position, max_len: usize) {
        self.answered = true;
        self.progressed = true;
        self.read_any = true;
        let file_count = self.files.len();
        let file = &mut self.files[self.index];
        let offset = u64::from(offset.get());
        let want = max_len.min((file.size - offset) as usize);
        let mut buf = vec![0u8; want];
        let read = file
            .reader
            .seek(SeekFrom::Start(offset))
            .and_then(|_| file.reader.read_exact(&mut buf));
        if let Err(e) = read {
            let msg = if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "the file got shorter while it was being sent".to_string()
            } else {
                e.to_string()
            };
            let msg = format!("could not read {}: {msg}", file.name);
            self.fail(UploadState::Failed(msg));
            return;
        }
        self.sent = offset + want as u64;
        self.state = UploadState::Sending {
            file_index: self.index,
            file_count,
            file_name: file.name.clone(),
            sent: self.sent,
            total: file.size,
        };
        if let Err(e) = self.sender.submit_file(&buf) {
            self.fail(UploadState::Failed(format!("ZMODEM: {e}")));
        }
    }

    /// Drops what is queued and sends the cancel sequence; `state` is
    /// reported once it is out.
    fn fail(&mut self, state: UploadState) {
        self.out.clear();
        self.abort_state = Some(state);
        self.need_start = false;
        self.need_finish = false;
        self.sender.abort();
    }

    fn finishing(&self) -> bool {
        self.state.is_final() || self.abort_state.is_some()
    }
}

impl Upload for ZmodemUpload {
    fn receive(&mut self, data: &[u8]) {
        if self.state.is_final() {
            self.leftover.extend_from_slice(data);
            return;
        }
        for (i, &b) in data.iter().enumerate() {
            self.cans = if b == CAN { self.cans + 1 } else { 0 };
            if self.cans >= CANCEL_RUN && !self.finishing() {
                // `zmodem2` only knows the ZABORT/ZCAN headers.
                self.inbuf.clear();
                self.leftover.extend_from_slice(&data[i + 1..]);
                self.fail(UploadState::Failed(
                    "the device cancelled the transfer".into(),
                ));
                self.pump();
                return;
            }
        }
        self.inbuf.extend_from_slice(data);
        self.pump();
    }

    fn transmit(&mut self, out: &mut Vec<u8>) {
        self.pump();
        let n = self.out.len().min(TRANSMIT_CHUNK);
        if n > 0 {
            self.progressed = true;
        }
        out.extend(self.out.drain(..n));
    }

    fn tick(&mut self, now: Instant) {
        self.now = now;
        if self.progressed {
            self.last_progress = now;
            self.progressed = false;
        }
        if self.finishing() {
            return;
        }
        if !self.answered {
            if now.duration_since(self.started) >= HANDSHAKE_TIMEOUT {
                self.fail(UploadState::Failed(
                    "the device did not answer rz (is lrzsz installed?)".into(),
                ));
            }
        } else if now.duration_since(self.last_progress) >= STALL_TIMEOUT {
            self.fail(UploadState::Failed(
                "the transfer stalled (no progress for 60 s)".into(),
            ));
        }
        self.pump();
    }

    fn cancel(&mut self) {
        if !self.finishing() {
            self.fail(UploadState::Cancelled);
            self.pump();
        }
    }

    fn state(&self) -> UploadState {
        self.state.clone()
    }

    fn leftover(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.leftover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::test_util::{bytes, drain, file};
    use zmodem2::Receiver;

    /// A `zmodem2` receiver standing in for `rz`.
    struct Peer {
        rx: Receiver,
        /// Names it refuses (ZSKIP), as `rz` does with files that exist.
        skip: Vec<&'static str>,
        pending: Vec<u8>,
        files: Vec<Received>,
        done: bool,
        /// Byte offset, over everything handed to the receiver so far, of a
        /// single byte to corrupt once: simulates line noise flipping a data
        /// subpacket. `fed` is where that running count is kept.
        corrupt_at: Option<usize>,
        fed: usize,
    }

    /// A file as the receiver got it: (name, size, modified, mode) and data.
    type Meta<'a> = (&'a str, Option<u32>, Option<u32>, Option<u32>);
    struct Received {
        name: String,
        size: Option<u32>,
        modified: Option<u32>,
        mode: Option<u32>,
        data: Vec<u8>,
    }

    impl Received {
        fn meta(&self) -> Meta<'_> {
            (&self.name, self.size, self.modified, self.mode)
        }
    }

    impl Peer {
        fn new(skip: &[&'static str]) -> Self {
            let mut rx = Receiver::new().unwrap();
            rx.set_manual_file_accept(true);
            Self {
                rx,
                skip: skip.to_vec(),
                pending: Vec::new(),
                files: Vec::new(),
                done: false,
                corrupt_at: None,
                fed: 0,
            }
        }

        /// One exchange each way; false when nothing moved.
        fn round(&mut self, up: &mut dyn Upload) -> bool {
            let mut out = drain(up);
            if let Some(pos) = self.corrupt_at {
                if pos >= self.fed && pos < self.fed + out.len() {
                    out[pos - self.fed] ^= 0xff;
                    self.corrupt_at = None;
                }
            }
            self.fed += out.len();
            let mut moved = !out.is_empty();
            self.pending.extend(out);
            while !self.pending.is_empty() {
                match self.rx.submit_wire(&self.pending) {
                    Ok(0) => break,
                    Ok(n) => {
                        self.pending.drain(..n);
                        moved = true;
                    }
                    Err(e) => panic!("receiver: {e:?}"),
                }
            }
            loop {
                match self.rx.poll() {
                    Action::WriteWire(b) => {
                        let n = b.len();
                        up.receive(b);
                        self.rx.wire_written(n);
                    }
                    Action::WriteFile(b) => {
                        let n = b.len();
                        self.files.last_mut().unwrap().data.extend_from_slice(b);
                        self.rx.file_written(n).unwrap();
                    }
                    Action::Event(Event::FileStarted(info)) => {
                        let name = String::from_utf8(info.name.to_vec()).unwrap();
                        let skip = self.skip.contains(&name.as_str());
                        self.files.push(Received {
                            name,
                            size: info.size.map(|s| s.get()),
                            modified: info.modified,
                            mode: info.mode,
                            data: Vec::new(),
                        });
                        if skip {
                            self.rx.skip_file().unwrap();
                        } else {
                            self.rx.accept_file_at(0).unwrap();
                        }
                    }
                    Action::Event(Event::SessionCompleted) => self.done = true,
                    Action::Event(_) => {}
                    _ => return moved,
                }
                moved = true;
            }
        }

        fn run(&mut self, up: &mut dyn Upload) {
            for _ in 0..100_000 {
                if !self.round(up) {
                    return;
                }
            }
            panic!("the upload did not finish");
        }

        /// Runs until the upload is part-way through a file.
        fn run_until_sending(&mut self, up: &mut dyn Upload) {
            for _ in 0..1000 {
                if let UploadState::Sending { sent, total, .. } = up.state() {
                    if sent > 0 && sent < total {
                        return;
                    }
                }
                self.round(up);
            }
            panic!("never got to sending: {:?}", up.state());
        }
    }

    #[test]
    fn sends_a_batch_with_names_times_and_modes() {
        let a = bytes(200_000, 1);
        let b = bytes(10, 2);
        let files = vec![
            file("a.bin", a.clone()),
            file("b.txt", b.clone()),
            file("empty", vec![]),
        ];
        let mut up = ZmodemUpload::new(files, Instant::now());
        // The shell's echo of `rz -b` and rz's banner come first.
        up.receive(b"rz -b\r\nrz waiting to receive.");
        let mut peer = Peer::new(&[]);
        peer.run(&mut up);
        assert!(peer.done);
        let meta: Vec<_> = peer.files.iter().map(Received::meta).collect();
        let (t, m) = (Some(1_700_000_000), Some(FILE_MODE));
        assert_eq!(
            meta,
            vec![
                ("a.bin", Some(200_000), t, m),
                ("b.txt", Some(10), t, m),
                ("empty", Some(0), t, m)
            ]
        );
        assert_eq!(peer.files[0].data, a);
        assert_eq!(peer.files[1].data, b);
        assert!(peer.files[2].data.is_empty());
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 3,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_file_the_receiver_skips_is_counted_as_skipped() {
        let files = vec![
            file("old.bin", bytes(5000, 3)),
            file("new.bin", bytes(5000, 4)),
        ];
        let mut up = ZmodemUpload::new(files, Instant::now());
        let mut peer = Peer::new(&["old.bin"]);
        peer.run(&mut up);
        assert!(peer.done);
        assert!(peer.files[0].data.is_empty());
        assert_eq!(peer.files[1].data.len(), 5000);
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 1
            }
        );
    }

    #[test]
    fn reports_progress_while_sending() {
        let mut up = ZmodemUpload::new(vec![file("a.bin", bytes(100_000, 5))], Instant::now());
        Peer::new(&[]).run_until_sending(&mut up);
        match up.state() {
            UploadState::Sending {
                file_index,
                file_count,
                file_name,
                total,
                ..
            } => {
                assert_eq!(
                    (file_index, file_count, file_name.as_str(), total),
                    (0, 1, "a.bin", 100_000)
                );
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn bytes_after_the_session_are_leftover() {
        let mut up = ZmodemUpload::new(vec![file("a", vec![1, 2, 3])], Instant::now());
        Peer::new(&[]).run(&mut up);
        assert!(up.state().is_final());
        up.receive(b"$ ");
        // The tail of rz's last header (CR LF) may come first.
        assert!(up.leftover().ends_with(b"$ "));
    }

    #[test]
    fn no_answer_from_rz_fails_and_cancels() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", vec![1])], t);
        assert!(drain(&mut up).is_empty(), "no ZRQINIT until rz answers");
        up.tick(t + Duration::from_secs(2));
        assert!(drain(&mut up).is_empty(), "still nothing to send");
        up.tick(t + HANDSHAKE_TIMEOUT);
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        match up.state() {
            UploadState::Failed(m) => assert!(m.contains("lrzsz"), "{m}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn a_shell_echoing_everything_back_does_not_loop() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(1000, 17))], t);
        // As if `rz` were missing and the shell just rejected the command.
        up.receive(b"rz: command not found\r\n$ ");
        let mut sent = Vec::new();
        let mut elapsed = Duration::ZERO;
        while elapsed < Duration::from_secs(12) && !up.state().is_final() {
            let out = drain(&mut up);
            sent.extend_from_slice(&out);
            // A cooked-mode shell echoes back whatever we just sent it.
            up.receive(&out);
            elapsed += Duration::from_millis(100);
            up.tick(t + elapsed);
        }
        assert!(
            !sent
                .windows(ZRQINIT_PREFIX.len())
                .any(|w| w == ZRQINIT_PREFIX),
            "no ZRQINIT ever went out"
        );
        assert_eq!(elapsed, HANDSHAKE_TIMEOUT, "failed right at the timeout");
        match up.state() {
            UploadState::Failed(m) => assert!(m.contains("lrzsz"), "{m}"),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn cancel_sends_the_cancel_sequence() {
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 6))], Instant::now());
        Peer::new(&[]).run_until_sending(&mut up);
        up.cancel();
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        assert_eq!(up.state(), UploadState::Cancelled);
        up.receive(b"$ ");
        assert_eq!(up.leftover(), b"$ ");
    }

    #[test]
    fn the_receiver_cancelling_fails_the_upload() {
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 7))], Instant::now());
        let mut peer = Peer::new(&[]);
        peer.run_until_sending(&mut up);
        // What `rz` sends on Ctrl+C: CANs, backspaces; then the shell prompt.
        let mut cancel = vec![CAN; 10];
        cancel.extend_from_slice(&[0x08; 10]);
        cancel.extend_from_slice(b"$ ");
        up.receive(&cancel);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device cancelled the transfer".into())
        );
        let mut rest = vec![CAN; 5];
        rest.extend_from_slice(&[0x08; 10]);
        rest.extend_from_slice(b"$ ");
        assert_eq!(up.leftover(), rest, "everything after the fifth CAN");
        assert!(
            !drain(&mut up).contains(&b'*'),
            "no more frames after the cancel"
        );
    }

    #[test]
    fn no_progress_after_the_handshake_fails() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 8))], t);
        Peer::new(&[]).run_until_sending(&mut up);
        // Settle whatever progress getting to `Sending` itself made, so
        // `last_progress` starts exactly at `t`.
        up.tick(t);
        up.tick(t + STALL_TIMEOUT - Duration::from_millis(1));
        assert!(!up.state().is_final());
        up.tick(t + STALL_TIMEOUT);
        assert_eq!(
            up.state(),
            UploadState::Failed("the transfer stalled (no progress for 60 s)".into())
        );
    }

    #[test]
    fn rz_retries_are_not_progress() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 12))], t);
        Peer::new(&[]).run_until_sending(&mut up);
        up.tick(t);
        // The handshake header `rz` keeps re-sending for minutes; `zmodem2`
        // ignores it once past the handshake (see `STALL_TIMEOUT`), so this
        // must not look like progress.
        let mut rx = Receiver::new().unwrap();
        let retry = match rx.poll() {
            Action::WriteWire(b) => b.to_vec(),
            a => panic!("{a:?}"),
        };
        for i in 1..=5u32 {
            up.receive(&retry);
            up.tick(t + Duration::from_secs(10) * i);
            assert!(!up.state().is_final(), "{i}: {:?}", up.state());
        }
        up.receive(&retry);
        up.tick(t + STALL_TIMEOUT);
        assert_eq!(
            up.state(),
            UploadState::Failed("the transfer stalled (no progress for 60 s)".into())
        );
    }

    #[test]
    fn a_slow_link_that_keeps_draining_is_not_a_stall() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(500_000, 11))], t);
        let mut peer = Peer::new(&[]);
        peer.run_until_sending(&mut up);
        up.tick(t);
        // Each round trip takes 50 s, under the 60 s stall timeout, so the
        // slow link never looks stalled even after minutes.
        for i in 1..=4u32 {
            assert!(peer.round(&mut up), "round {i}: nothing moved");
            up.tick(t + Duration::from_secs(50) * i);
            assert!(!up.state().is_final(), "round {i}: {:?}", up.state());
        }
    }

    #[test]
    fn a_late_tick_after_data_does_not_fail() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 10))], t);
        let mut peer = Peer::new(&[]);
        peer.run_until_sending(&mut up);
        up.tick(t);
        assert!(!up.state().is_final());
        // Real progress, soon after the first tick.
        peer.round(&mut up);
        // No tick comes in for 10 minutes (e.g. the host slept); the first
        // tick back must stamp `last_progress` at its own `now`, not fail
        // because the real progress was long ago.
        up.tick(t + Duration::from_secs(600));
        assert!(!up.state().is_final(), "{:?}", up.state());
        up.tick(t + Duration::from_secs(600) + STALL_TIMEOUT);
        assert_eq!(
            up.state(),
            UploadState::Failed("the transfer stalled (no progress for 60 s)".into())
        );
    }

    #[test]
    fn the_handshake_counts_once_zfile_goes_out() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 9))], t);
        let mut rx = Receiver::new().unwrap();
        let zrinit = match rx.poll() {
            Action::WriteWire(b) => b.to_vec(),
            a => panic!("{a:?}"),
        };
        up.tick(t + Duration::from_millis(9_500));
        up.receive(&zrinit);
        assert!(!drain(&mut up).is_empty(), "ZFILE should go out");
        // Before the fix this would fail with "did not answer rz": ZRINIT
        // was seen, but neither ReadFile nor FileCompleted had happened yet.
        up.tick(t + Duration::from_millis(10_500));
        assert!(!up.state().is_final(), "{:?}", up.state());
    }

    #[test]
    fn a_cancel_split_across_reads_is_seen() {
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 16))], Instant::now());
        up.receive(&[CAN; 3]);
        up.receive(&[CAN; 7]);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device cancelled the transfer".into())
        );
    }

    #[test]
    fn skipping_the_last_file_still_finishes() {
        let files = vec![
            file("keep.bin", bytes(2000, 13)),
            file("skip.bin", bytes(2000, 14)),
        ];
        let mut up = ZmodemUpload::new(files, Instant::now());
        let mut peer = Peer::new(&["skip.bin"]);
        peer.run(&mut up);
        assert!(peer.done);
        assert_eq!(peer.files[0].data.len(), 2000);
        assert!(peer.files[1].data.is_empty());
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 1
            }
        );
    }

    #[test]
    fn a_corrupted_subpacket_is_resent_from_the_receivers_offset() {
        let data = bytes(200_000, 15);
        let mut up = ZmodemUpload::new(vec![file("a.bin", data.clone())], Instant::now());
        let mut peer = Peer::new(&[]);
        // Somewhere in the middle of a data subpacket, well past the
        // handshake and ZFILE; the exact spot does not matter, flipping any
        // one byte breaks that subpacket's CRC and forces a ZRPOS rewind.
        peer.corrupt_at = Some(2000);
        peer.run(&mut up);
        assert!(peer.done);
        assert_eq!(peer.files[0].data, data);
        assert_eq!(
            up.state(),
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            }
        );
    }

    #[test]
    fn a_name_over_256_bytes_fails_clearly() {
        let name = "a".repeat(300);
        let f = file(&name, vec![1, 2, 3]);
        let mut up = ZmodemUpload::new(vec![f], Instant::now());
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        assert_eq!(
            up.state(),
            UploadState::Failed(format!("{name} has a name longer than ZMODEM's 256 bytes"))
        );
    }

    #[test]
    fn a_file_over_4_gib_fails_before_anything_is_sent() {
        let mut f = file("huge.img", vec![]);
        f.size = u64::from(u32::MAX) + 1;
        let mut up = ZmodemUpload::new(vec![f], Instant::now());
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
        assert_eq!(
            up.state(),
            UploadState::Failed("huge.img is larger than ZMODEM's 4 GiB limit".into())
        );
    }
}
