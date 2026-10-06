//! The session logs being written (plan N): one file per terminal pane or serial
//! tab that is recording. Fed at the raw byte entry points — the PTY tap and the
//! serial callback — so a log records whether or not the frontend is drawing.
//! Every stop, asked for or not, is reported once through `on_stopped`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use omnyssh_core::event::SessionId;
use omnyssh_core::session_log::{SessionLogger, NATIVE_NEWLINE};

/// A log being written reaches the disk at least this often: on the next chunk
/// after it, or on main.rs's tick while the session is quiet.
pub const FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// The write buffer. A full one goes to disk at once.
const BUFFER: usize = 64 * 1024;

/// How a log ended, for the `log-stopped` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    pub session_id: SessionId,
    pub path: String,
    /// Bytes written to the file, the header included.
    pub bytes: u64,
    /// Why writing failed; `None` for a log closed in good order.
    pub error: Option<String>,
}

struct SessionLog {
    writer: BufWriter<Box<dyn Write + Send>>,
    logger: SessionLogger,
    path: String,
    bytes: u64,
    /// Written since the last flush.
    dirty: bool,
    last_flush: Instant,
}

impl SessionLog {
    fn put(&mut self, text: &str) -> io::Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        self.writer.write_all(text.as_bytes())?;
        self.bytes += text.len() as u64;
        self.dirty = true;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()?;
        self.dirty = false;
        self.last_flush = Instant::now();
        Ok(())
    }

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let text = self.logger.feed(bytes, chrono::Local::now().naive_local());
        self.put(&text)?;
        if self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush()?;
        }
        Ok(())
    }

    fn stopped(&self, session_id: SessionId, error: Option<io::Error>) -> Stopped {
        Stopped {
            session_id,
            path: self.path.clone(),
            bytes: self.bytes,
            error: error.map(|e| e.to_string()),
        }
    }

    /// End the open line, flush, and close the file.
    fn close(mut self, session_id: SessionId) -> Stopped {
        let tail = self.logger.finish();
        let result = self.put(&tail).and_then(|()| self.flush());
        self.stopped(session_id, result.err())
    }
}

type Notify = Box<dyn Fn(Stopped) + Send + Sync>;

#[derive(Default)]
pub struct SessionLogs {
    logs: Mutex<HashMap<SessionId, SessionLog>>,
    notify: OnceLock<Notify>,
}

impl SessionLogs {
    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, SessionLog>> {
        self.logs.lock().expect("session logs lock poisoned")
    }

    /// Where stops are reported. Set once, at startup; called without a lock held.
    pub fn on_stopped(&self, f: impl Fn(Stopped) + Send + Sync + 'static) {
        let _ = self.notify.set(Box::new(f));
    }

    fn report(&self, stopped: Stopped) {
        if let Some(notify) = self.notify.get() {
            notify(stopped);
        }
    }

    pub fn is_logging(&self, id: SessionId) -> bool {
        self.lock().contains_key(&id)
    }

    /// Whether some session is currently writing a log to `path`. Checked before a
    /// second log or an export would otherwise truncate the file out from under its
    /// open writer.
    pub fn is_logging_path(&self, path: &Path) -> bool {
        self.lock().values().any(|log| Path::new(&log.path) == path)
    }

    /// Start logging `id` into `path`, replacing whatever the file held, with
    /// `header` as its first line.
    pub fn start(
        &self,
        id: SessionId,
        path: &Path,
        logger: SessionLogger,
        header: &str,
    ) -> Result<(), String> {
        // Checked before the file is truncated; `start_with` checks both again under
        // the lock.
        if self.is_logging(id) {
            return Err("This session is already being logged".to_string());
        }
        if self.is_logging_path(path) {
            return Err("That file is being logged to".to_string());
        }
        let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        self.start_with(
            id,
            path.to_string_lossy().into_owned(),
            Box::new(file),
            logger,
            header,
        )
    }

    fn start_with(
        &self,
        id: SessionId,
        path: String,
        sink: Box<dyn Write + Send>,
        logger: SessionLogger,
        header: &str,
    ) -> Result<(), String> {
        let mut log = SessionLog {
            writer: BufWriter::with_capacity(BUFFER, sink),
            logger,
            path,
            bytes: 0,
            dirty: false,
            last_flush: Instant::now(),
        };
        // Into the buffer only: it reaches the disk with the first flush.
        log.put(&format!("{header}{NATIVE_NEWLINE}"))
            .map_err(|e| e.to_string())?;
        let mut logs = self.lock();
        if logs.contains_key(&id) {
            return Err("This session is already being logged".to_string());
        }
        if logs.values().any(|l| l.path == log.path) {
            return Err("That file is being logged to".to_string());
        }
        logs.insert(id, log);
        Ok(())
    }

    /// Append a chunk of `id`'s output. A no-op unless `id` is being logged. A
    /// failed write stops the log, keeping what was written before it.
    pub fn write(&self, id: SessionId, bytes: &[u8]) {
        let failed = {
            let mut logs = self.lock();
            let Some(log) = logs.get_mut(&id) else {
                return;
            };
            match log.append(bytes) {
                Ok(()) => None,
                Err(e) => {
                    let stopped = log.stopped(id, Some(e));
                    logs.remove(&id);
                    Some(stopped)
                }
            }
        };
        if let Some(stopped) = failed {
            self.report(stopped);
        }
    }

    /// Flush every log written to since its last flush. Called once a second, so a
    /// quiet session's last lines reach the disk too.
    pub fn flush_idle(&self) {
        let mut failed = Vec::new();
        self.lock().retain(|&id, log| {
            if !log.dirty {
                return true;
            }
            match log.flush() {
                Ok(()) => true,
                Err(e) => {
                    failed.push(log.stopped(id, Some(e)));
                    false
                }
            }
        });
        for stopped in failed {
            self.report(stopped);
        }
    }

    /// Stop logging `id`: end the open line, flush and close the file. A no-op when
    /// it is not being logged.
    pub fn stop(&self, id: SessionId) {
        let log = self.lock().remove(&id);
        if let Some(log) = log {
            self.report(log.close(id));
        }
    }

    /// Stop every log, for the app quitting: the process exits without running
    /// destructors, so an unflushed buffer would be lost.
    pub fn stop_all(&self) {
        let logs: Vec<_> = self.lock().drain().collect();
        for (id, log) in logs {
            self.report(log.close(id));
        }
    }

    /// The file `id` is logged into and the bytes written to it so far.
    pub fn status(&self, id: SessionId) -> Option<(String, u64)> {
        self.lock()
            .get(&id)
            .map(|log| (log.path.clone(), log.bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnyssh_core::session_log::{HexLogger, TextLogger};
    use std::sync::Arc;

    fn text() -> SessionLogger {
        SessionLogger::Text(TextLogger::new(false, NATIVE_NEWLINE))
    }

    /// Logs whose stops land in the returned list.
    fn logs() -> (SessionLogs, Arc<Mutex<Vec<Stopped>>>) {
        let logs = SessionLogs::default();
        let stops = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&stops);
        logs.on_stopped(move |s| sink.lock().unwrap().push(s));
        (logs, stops)
    }

    /// `\n` written as this platform's newline.
    fn native(s: &str) -> String {
        s.replace('\n', NATIVE_NEWLINE)
    }

    #[test]
    fn a_log_holds_the_header_and_the_stripped_output() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web-1.log");
        let (logs, stops) = logs();

        logs.start(7, &path, text(), "# OmnySSH log: web-1")
            .unwrap();
        assert!(logs.is_logging(7));
        logs.write(7, b"\x1b[32m$\x1b[0m ls\r\nREADME.md\r\n$ ");
        // Another session's output is not this log's.
        logs.write(8, b"elsewhere\r\n");
        let (shown, bytes) = logs.status(7).unwrap();
        assert_eq!(shown, path.to_string_lossy());
        logs.stop(7);

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            written,
            native("# OmnySSH log: web-1\n$ ls\nREADME.md\n$ \n")
        );
        assert!(!logs.is_logging(7));
        assert_eq!(logs.status(7), None);
        let stops = stops.lock().unwrap();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].session_id, 7);
        assert_eq!(stops[0].error, None);
        assert_eq!(stops[0].bytes, written.len() as u64);
        assert!(bytes < stops[0].bytes, "the closing newline came last");
    }

    #[test]
    fn starting_truncates_an_existing_file_and_a_second_start_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.log");
        std::fs::write(&path, "an older log that was much longer").unwrap();
        let (logs, _) = logs();

        logs.start(1, &path, text(), "# new").unwrap();
        assert!(logs.start(1, &path, text(), "# again").is_err());
        logs.stop(1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), native("# new\n"));
    }

    #[test]
    fn a_second_session_cannot_start_a_log_on_a_path_already_being_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.log");
        let (logs, _) = logs();

        logs.start(1, &path, text(), "# one").unwrap();
        assert!(!logs.is_logging_path(&dir.path().join("other.log")));
        assert!(logs.is_logging_path(&path));
        let err = logs.start(2, &path, text(), "# two").unwrap_err();
        assert_eq!(err, "That file is being logged to");
        assert!(!logs.is_logging(2));

        // Freed once the first session's log is stopped.
        logs.stop(1);
        assert!(!logs.is_logging_path(&path));
        logs.start(2, &path, text(), "# two").unwrap();
        logs.stop(2);
    }

    #[test]
    fn a_file_that_cannot_be_created_fails_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let (logs, stops) = logs();
        let err = logs
            .start(1, &dir.path().join("missing/dir/x.log"), text(), "#")
            .unwrap_err();
        assert!(err.contains("x.log"), "{err}");
        assert!(!logs.is_logging(1));
        assert!(stops.lock().unwrap().is_empty());
    }

    #[test]
    fn a_hex_log_dumps_the_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("COM3.log");
        let (logs, _) = logs();
        logs.start(
            2,
            &path,
            SessionLogger::Hex(HexLogger::new(NATIVE_NEWLINE)),
            "# COM3",
        )
        .unwrap();
        logs.write(2, &[0x55, 0xaa, 0x01]);
        logs.stop(2);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            native("# COM3\n55 AA 01\n")
        );
    }

    #[test]
    fn quiet_output_reaches_the_disk_on_the_idle_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quiet.log");
        let (logs, stops) = logs();
        logs.start(3, &path, text(), "# quiet").unwrap();
        logs.write(3, b"one line\n");
        // Still in the buffer: under a second has passed and it is far from full.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");

        logs.flush_idle();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            native("# quiet\none line\n")
        );
        assert!(logs.is_logging(3));
        assert!(stops.lock().unwrap().is_empty());
    }

    #[test]
    fn a_full_buffer_goes_to_disk_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("busy.log");
        let (logs, _) = logs();
        logs.start(4, &path, text(), "#").unwrap();
        logs.write(4, &[b'x'; BUFFER + 1]);
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        logs.stop(4);
    }

    /// Takes `room` bytes, then fails the way a full disk does.
    struct Disk {
        room: usize,
    }

    impl Write for Disk {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.room == 0 {
                return Err(io::Error::other("No space left on device"));
            }
            let n = buf.len().min(self.room);
            self.room -= n;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_write_stops_the_log_and_says_why() {
        let (logs, stops) = logs();
        logs.start_with(
            5,
            "full.log".to_string(),
            Box::new(Disk { room: 10 }),
            text(),
            "#",
        )
        .unwrap();
        logs.write(5, b"fits\n");
        assert!(logs.is_logging(5));
        // Bigger than the buffer, so it is written through at once — and fails.
        logs.write(5, &[b'x'; BUFFER + 1]);

        assert!(!logs.is_logging(5));
        let stops = stops.lock().unwrap();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].session_id, 5);
        assert_eq!(stops[0].path, "full.log");
        assert_eq!(stops[0].error.as_deref(), Some("No space left on device"));
        drop(stops);
        // Later output for the session is ignored.
        logs.write(5, b"more\n");
        assert_eq!(logs.status(5), None);
    }

    #[test]
    fn a_failed_idle_flush_stops_the_log() {
        let (logs, stops) = logs();
        logs.start_with(
            6,
            "full.log".to_string(),
            Box::new(Disk { room: 0 }),
            text(),
            "#",
        )
        .unwrap();
        logs.flush_idle();
        assert!(!logs.is_logging(6));
        assert!(stops.lock().unwrap()[0].error.is_some());
    }

    #[test]
    fn quitting_closes_every_log() {
        let dir = tempfile::tempdir().unwrap();
        let (logs, stops) = logs();
        for id in [1, 2] {
            logs.start(id, &dir.path().join(format!("{id}.log")), text(), "#")
                .unwrap();
            logs.write(id, b"partial line");
        }
        logs.stop_all();
        for id in [1, 2] {
            assert_eq!(
                std::fs::read_to_string(dir.path().join(format!("{id}.log"))).unwrap(),
                native("#\npartial line\n")
            );
        }
        assert_eq!(stops.lock().unwrap().len(), 2);
        // Stopping what is no longer logged reports nothing more.
        logs.stop(1);
        assert_eq!(stops.lock().unwrap().len(), 2);
    }
}
