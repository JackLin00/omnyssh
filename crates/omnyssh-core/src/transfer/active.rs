//! One upload inside a session loop: the part the serial thread and the SSH
//! pump share. It starts the protocol, throttles progress reports, decides
//! when the session may leave upload mode, and writes the result line the
//! terminal shows (and the session log records) at the end.

use std::time::{Duration, Instant};

use super::{start, Protocol, Upload, UploadFile, UploadState};

/// Progress goes out at most this often.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// How long an upload that has ended may keep sending what it still has
/// queued (a cancel sequence, ZMODEM's `OO`) before the session drops it. A
/// stalled line would otherwise keep the session in upload mode for good.
const DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// How an upload ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadResult {
    pub ok: bool,
    /// `uploaded foo.bin (1.2 MB)`, `cancelled`, or why it failed.
    pub message: String,
}

impl UploadResult {
    /// The line the terminal shows at the end, starting on a fresh line: what
    /// the device sent last may have left the cursor anywhere.
    pub fn line(&self) -> String {
        if self.ok || self.message == CANCELLED {
            format!("\r\n[upload] {}\r\n", self.message)
        } else {
            format!("\r\n[upload] failed: {}\r\n", self.message)
        }
    }
}

const CANCELLED: &str = "cancelled";

/// What a session reports about its upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadEvent {
    /// Waiting for the receiver, or sending; never a final state.
    Progress(UploadState),
    Finished(UploadResult),
}

/// The end of an upload: what to show in the terminal (the device's last
/// output, then the result line), and the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub output: Vec<u8>,
    pub result: UploadResult,
}

pub struct ActiveUpload {
    upload: Box<dyn Upload>,
    /// Name and size of each file, for the result line.
    files: Vec<(String, u64)>,
    reported: Option<UploadState>,
    reported_at: Option<Instant>,
    /// Set when the upload is first seen ended.
    drain_until: Option<Instant>,
    /// ZMODEM: an upload that ends in failure or cancellation drops what is
    /// queued, so its cancel sequence is not stuck behind file data.
    discard_on_end: bool,
}

impl ActiveUpload {
    /// Starts sending `files`. The caller has checked them first: not empty,
    /// one file for XMODEM (see [`start`]).
    pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Self {
        let names = files.iter().map(|f| (f.name.clone(), f.size)).collect();
        Self::new(
            start(protocol, files, now),
            names,
            protocol == Protocol::Zmodem,
        )
    }

    fn new(upload: Box<dyn Upload>, files: Vec<(String, u64)>, discard_on_end: bool) -> Self {
        Self {
            upload,
            files,
            reported: None,
            reported_at: None,
            drain_until: None,
            discard_on_end,
        }
    }

    /// Bytes the device sent.
    pub fn receive(&mut self, data: &[u8], now: Instant) {
        self.upload.tick(now);
        self.upload.receive(data);
    }

    /// Call regularly (every 100 ms or so) while uploading.
    pub fn tick(&mut self, now: Instant) {
        self.upload.tick(now);
    }

    /// The user cancelled. The cancel sequence goes out through `pump`.
    pub fn cancel(&mut self) {
        self.upload.cancel();
    }

    /// Moves the upload along. When `pending` (the session's unsent bytes) is
    /// empty, fills it with what to send next. Returns `Some` once the upload
    /// has ended and everything was sent, or `DRAIN_LIMIT` has passed since it
    /// ended; `pending` is then cleared, and the session leaves upload mode.
    pub fn pump(&mut self, now: Instant, pending: &mut Vec<u8>) -> Option<Finished> {
        // On a slow line `pending` can hold seconds of file data. ZMODEM's
        // cancel sequence works mid-frame, so when the upload first ends
        // badly the stale data goes, and the sequence below is sent next.
        // (XMODEM/YMODEM keep their block: a cancel must not land inside it.)
        if self.drain_until.is_none() && self.discard_on_end {
            let state = self.upload.state();
            if state.is_final() && !matches!(state, UploadState::Done { .. }) {
                pending.clear();
            }
        }
        if pending.is_empty() {
            self.upload.transmit(pending);
        }
        let state = self.upload.state();
        if !state.is_final() {
            return None;
        }
        let until = *self.drain_until.get_or_insert(now + DRAIN_LIMIT);
        if !pending.is_empty() && now < until {
            return None;
        }
        pending.clear();
        let result = self.result(&state);
        let mut output = self.upload.leftover();
        output.extend_from_slice(result.line().as_bytes());
        Some(Finished { output, result })
    }

    /// The state to report, if it changed and the last report is
    /// `PROGRESS_INTERVAL` old. Final states are reported by `pump` instead.
    pub fn progress(&mut self, now: Instant) -> Option<UploadState> {
        let state = self.upload.state();
        if state.is_final() || self.reported.as_ref() == Some(&state) {
            return None;
        }
        if self
            .reported_at
            .is_some_and(|at| now.duration_since(at) < PROGRESS_INTERVAL)
        {
            return None;
        }
        self.reported = Some(state.clone());
        self.reported_at = Some(now);
        Some(state)
    }

    /// The connection ended in the middle of the upload.
    pub fn lost(mut self) -> Finished {
        let state = self.upload.state();
        let result = if state.is_final() {
            self.result(&state)
        } else {
            UploadResult {
                ok: false,
                message: "the connection was lost".to_string(),
            }
        };
        let mut output = self.upload.leftover();
        output.extend_from_slice(result.line().as_bytes());
        Finished { output, result }
    }

    fn result(&self, state: &UploadState) -> UploadResult {
        match state {
            UploadState::Done {
                sent_files,
                skipped,
            } => UploadResult {
                ok: true,
                message: self.done_message(*sent_files, *skipped),
            },
            UploadState::Failed(reason) => UploadResult {
                ok: false,
                message: reason.clone(),
            },
            _ => UploadResult {
                ok: false,
                message: CANCELLED.to_string(),
            },
        }
    }

    fn done_message(&self, sent: usize, skipped: usize) -> String {
        let total: u64 = self.files.iter().map(|(_, size)| size).sum();
        match self.files.as_slice() {
            [(name, size)] if skipped == 0 => format!("uploaded {name} ({})", human_size(*size)),
            [(name, _)] => format!("{name} was skipped by the receiver"),
            files if skipped == 0 => {
                format!("uploaded {} files ({})", files.len(), human_size(total))
            }
            files => format!(
                "uploaded {sent} of {} files, {skipped} skipped by the receiver",
                files.len()
            ),
        }
    }
}

/// `512 B`, `1.2 KB`, `3.4 MB`, `1.0 GB` (powers of 1024).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    // 1023.95 and up would print as "1024.0".
    while value >= 1023.95 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// An upload whose state and output the test sets.
    #[derive(Default)]
    struct Script {
        state: Option<UploadState>,
        to_send: Vec<Vec<u8>>,
        leftover: Vec<u8>,
        received: Vec<u8>,
        ticks: usize,
        cancelled: bool,
    }

    struct Fake(Arc<Mutex<Script>>);

    impl Upload for Fake {
        fn receive(&mut self, data: &[u8]) {
            self.0.lock().unwrap().received.extend_from_slice(data);
        }
        fn transmit(&mut self, out: &mut Vec<u8>) {
            let mut s = self.0.lock().unwrap();
            if !s.to_send.is_empty() {
                let chunk = s.to_send.remove(0);
                out.extend_from_slice(&chunk);
            }
        }
        fn tick(&mut self, _now: Instant) {
            self.0.lock().unwrap().ticks += 1;
        }
        fn cancel(&mut self) {
            let mut s = self.0.lock().unwrap();
            s.cancelled = true;
            s.state = Some(UploadState::Cancelled);
            s.to_send = vec![b"CANCEL".to_vec()];
        }
        fn state(&self) -> UploadState {
            self.0
                .lock()
                .unwrap()
                .state
                .clone()
                .unwrap_or(UploadState::Waiting)
        }
        fn leftover(&mut self) -> Vec<u8> {
            std::mem::take(&mut self.0.lock().unwrap().leftover)
        }
    }

    fn fake(files: &[(&str, u64)]) -> (ActiveUpload, Arc<Mutex<Script>>) {
        fake_with(files, false)
    }

    fn fake_zmodem(files: &[(&str, u64)]) -> (ActiveUpload, Arc<Mutex<Script>>) {
        fake_with(files, true)
    }

    fn fake_with(files: &[(&str, u64)], discard: bool) -> (ActiveUpload, Arc<Mutex<Script>>) {
        let script = Arc::new(Mutex::new(Script::default()));
        let files = files.iter().map(|(n, s)| (n.to_string(), *s)).collect();
        (
            ActiveUpload::new(Box::new(Fake(Arc::clone(&script))), files, discard),
            script,
        )
    }

    fn sending(sent: u64) -> UploadState {
        UploadState::Sending {
            file_index: 0,
            file_count: 1,
            file_name: "a.bin".to_string(),
            sent,
            total: 100,
        }
    }

    #[test]
    fn receive_ticks_before_handing_the_bytes_over() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        up.receive(b"C", Instant::now());
        let s = script.lock().unwrap();
        assert_eq!(s.ticks, 1);
        assert_eq!(s.received, b"C");
    }

    #[test]
    fn pump_fills_only_an_empty_buffer() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        script.lock().unwrap().to_send = vec![b"one".to_vec(), b"two".to_vec()];
        let now = Instant::now();
        let mut pending = b"unsent".to_vec();
        assert_eq!(up.pump(now, &mut pending), None);
        assert_eq!(pending, b"unsent", "nothing added while bytes wait");
        pending.clear();
        assert_eq!(up.pump(now, &mut pending), None);
        assert_eq!(pending, b"one");
    }

    #[test]
    fn a_finished_upload_ends_once_everything_is_sent() {
        let (mut up, script) = fake(&[("a.bin", 2048)]);
        {
            let mut s = script.lock().unwrap();
            s.state = Some(UploadState::Done {
                sent_files: 1,
                skipped: 0,
            });
            s.to_send = vec![b"OO".to_vec()];
            s.leftover = b"$ ".to_vec();
        }
        let now = Instant::now();
        let mut pending = Vec::new();
        assert_eq!(up.pump(now, &mut pending), None, "OO goes out first");
        assert_eq!(pending, b"OO");
        pending.clear();
        let end = up.pump(now, &mut pending).expect("ended");
        assert_eq!(
            end.result,
            UploadResult {
                ok: true,
                message: "uploaded a.bin (2.0 KB)".to_string()
            }
        );
        assert_eq!(end.output, b"$ \r\n[upload] uploaded a.bin (2.0 KB)\r\n");
    }

    #[test]
    fn a_stalled_line_is_given_up_on_after_the_drain_limit() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        up.cancel();
        assert!(script.lock().unwrap().cancelled);
        let t = Instant::now();
        let mut pending = Vec::new();
        assert_eq!(up.pump(t, &mut pending), None);
        assert_eq!(pending, b"CANCEL");
        // The line takes nothing.
        assert_eq!(up.pump(t + Duration::from_secs(1), &mut pending), None);
        let end = up.pump(t + DRAIN_LIMIT, &mut pending).expect("given up");
        assert!(pending.is_empty(), "the unsent rest is dropped");
        assert_eq!(end.result.message, "cancelled");
        assert_eq!(end.output, b"\r\n[upload] cancelled\r\n");
    }

    #[test]
    fn a_failure_reads_as_failed_with_its_reason() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        script.lock().unwrap().state = Some(UploadState::Failed("too many retries".to_string()));
        let end = up.pump(Instant::now(), &mut Vec::new()).unwrap();
        assert!(!end.result.ok);
        assert_eq!(end.output, b"\r\n[upload] failed: too many retries\r\n");
    }

    #[test]
    fn results_name_the_files() {
        let (up, _) = fake(&[("a.bin", 100)]);
        assert_eq!(up.done_message(0, 1), "a.bin was skipped by the receiver");
        let (up, _) = fake(&[("a", 1024), ("b", 1024), ("c", 1024)]);
        assert_eq!(up.done_message(3, 0), "uploaded 3 files (3.0 KB)");
        assert_eq!(
            up.done_message(2, 1),
            "uploaded 2 of 3 files, 1 skipped by the receiver"
        );
    }

    #[test]
    fn progress_is_reported_when_it_changes_at_most_every_100_ms() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        let t = Instant::now();
        assert_eq!(up.progress(t), Some(UploadState::Waiting));
        assert_eq!(up.progress(t + PROGRESS_INTERVAL), None, "unchanged");
        script.lock().unwrap().state = Some(sending(10));
        assert_eq!(up.progress(t + Duration::from_millis(50)), None, "too soon");
        assert_eq!(up.progress(t + PROGRESS_INTERVAL), Some(sending(10)));
        script.lock().unwrap().state = Some(UploadState::Cancelled);
        assert_eq!(
            up.progress(t + 10 * PROGRESS_INTERVAL),
            None,
            "final states end through pump"
        );
    }

    #[test]
    fn a_lost_connection_fails_the_upload() {
        let (up, script) = fake(&[("a.bin", 100)]);
        script.lock().unwrap().leftover = b"partial".to_vec();
        let end = up.lost();
        assert!(!end.result.ok);
        assert_eq!(
            end.output,
            b"partial\r\n[upload] failed: the connection was lost\r\n"
        );
    }

    #[test]
    fn sizes_read_in_binary_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(1023), "1023 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(1_258_291), "1.2 MB");
        assert_eq!(human_size(5 << 30), "5.0 GB");
        assert_eq!(human_size(1_048_575), "1.0 MB");
        assert_eq!(human_size((1 << 30) - 1), "1.0 GB");
        assert_eq!(human_size(3 << 40), "3.0 TB");
    }

    #[test]
    fn a_zmodem_cancel_drops_queued_file_data() {
        let (mut up, _script) = fake_zmodem(&[("a.bin", 100)]);
        up.cancel();
        let mut pending = b"stale file data".to_vec();
        assert_eq!(up.pump(Instant::now(), &mut pending), None);
        assert_eq!(pending, b"CANCEL");
    }

    #[test]
    fn an_xmodem_cancel_keeps_the_block_in_flight() {
        let (mut up, _script) = fake(&[("a.bin", 100)]);
        up.cancel();
        let mut pending = b"block".to_vec();
        assert_eq!(up.pump(Instant::now(), &mut pending), None);
        assert_eq!(pending, b"block");
    }

    #[test]
    fn a_finished_zmodem_upload_keeps_what_is_queued() {
        let (mut up, script) = fake_zmodem(&[("a.bin", 100)]);
        script.lock().unwrap().state = Some(UploadState::Done {
            sent_files: 1,
            skipped: 0,
        });
        let mut pending = b"tail".to_vec();
        assert_eq!(up.pump(Instant::now(), &mut pending), None);
        assert_eq!(pending, b"tail");
    }

    #[test]
    fn a_connection_lost_after_the_end_keeps_the_result() {
        let (up, script) = fake(&[("a.bin", 2048)]);
        script.lock().unwrap().state = Some(UploadState::Done {
            sent_files: 1,
            skipped: 0,
        });
        let end = up.lost();
        assert!(end.result.ok, "{:?}", end.result);
        assert_eq!(end.result.message, "uploaded a.bin (2.0 KB)");
    }

    #[test]
    fn the_drain_limit_also_ends_a_finished_upload_on_a_stalled_line() {
        let (mut up, script) = fake(&[("a.bin", 100)]);
        script.lock().unwrap().state = Some(UploadState::Done {
            sent_files: 1,
            skipped: 0,
        });
        let t = Instant::now();
        let mut pending = b"OO".to_vec();
        assert_eq!(up.pump(t, &mut pending), None);
        assert_eq!(
            up.pump(t + DRAIN_LIMIT - Duration::from_millis(1), &mut pending),
            None
        );
        let end = up.pump(t + DRAIN_LIMIT, &mut pending).expect("gave up");
        assert!(end.result.ok);
        assert!(pending.is_empty());
    }

    #[test]
    fn a_real_protocol_runs_through_the_driver() {
        // XMODEM to a receiver that only ever says C then ACKs.
        let file = UploadFile {
            name: "a.bin".to_string(),
            size: 128,
            modified: None,
            reader: Box::new(std::io::Cursor::new(vec![7u8; 128])),
        };
        let t = Instant::now();
        let mut up = ActiveUpload::start(Protocol::Xmodem, vec![file], t);
        let mut pending = Vec::new();
        up.receive(b"C", t);
        assert_eq!(up.pump(t, &mut pending), None);
        assert_eq!(pending[0], 0x01, "a 128-byte block (SOH)");
        pending.clear();
        up.receive(&[0x06], t); // ACK the block
        assert_eq!(up.pump(t, &mut pending), None);
        assert_eq!(pending, [0x04], "EOT");
        pending.clear();
        up.receive(&[0x06], t);
        let end = up.pump(t, &mut pending).expect("done");
        assert!(end.result.ok, "{:?}", end.result);
    }
}
