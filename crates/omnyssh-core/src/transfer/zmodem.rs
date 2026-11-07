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
/// How often ZRQINIT goes out again while waiting for ZRINIT.
const ZRQINIT_INTERVAL: Duration = Duration::from_secs(2);
/// Silence from the receiver, once it has answered, before giving up. It
/// acknowledges every few subpackets, so this only passes on a dead link.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// A regular file readable by all, writable by its owner: without a mode `rz`
/// creates the file `0600`.
const FILE_MODE: u32 = 0o100_644;
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
    started: Instant,
    last_zrqinit: Instant,
    last_rx: Instant,
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
            started: now,
            last_zrqinit: now,
            last_rx: now,
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
                    self.out.extend_from_slice(bytes);
                    self.sender.wire_written(n);
                }
                Action::ReadFile { offset, max_len } => self.read_file(offset, max_len),
                Action::Event(Event::FileCompleted) => {
                    self.answered = true;
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
        self.last_rx = self.now;
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
        out.extend(self.out.drain(..n));
    }

    fn tick(&mut self, now: Instant) {
        self.now = now;
        if self.finishing() {
            return;
        }
        if !self.answered {
            if now.duration_since(self.started) >= HANDSHAKE_TIMEOUT {
                self.fail(UploadState::Failed(
                    "the device did not answer rz (is lrzsz installed?)".into(),
                ));
            } else if now.duration_since(self.last_zrqinit) >= ZRQINIT_INTERVAL {
                self.last_zrqinit = now;
                let _ = self.sender.timeout();
            }
        } else if now.duration_since(self.last_rx) >= IDLE_TIMEOUT {
            self.fail(UploadState::Failed("the device stopped responding".into()));
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
            }
        }

        /// One exchange each way; false when nothing moved.
        fn round(&mut self, up: &mut dyn Upload) -> bool {
            let out = drain(up);
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
        let first = drain(&mut up);
        assert!(first.starts_with(b"**\x18B00"), "ZRQINIT first");
        up.tick(t + ZRQINIT_INTERVAL);
        assert_eq!(drain(&mut up), first, "ZRQINIT again");
        up.tick(t + HANDSHAKE_TIMEOUT);
        assert!(drain(&mut up).starts_with(&[CAN; 10]));
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
    fn silence_after_the_handshake_fails() {
        let t = Instant::now();
        let mut up = ZmodemUpload::new(vec![file("a", bytes(50_000, 8))], t);
        Peer::new(&[]).run_until_sending(&mut up);
        up.tick(t + IDLE_TIMEOUT - Duration::from_millis(1));
        assert!(!up.state().is_final());
        up.tick(t + IDLE_TIMEOUT);
        assert_eq!(
            up.state(),
            UploadState::Failed("the device stopped responding".into())
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
