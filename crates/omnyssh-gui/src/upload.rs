//! Drag-and-drop uploads, the parts that need no Tauri: the checks a drop goes
//! through before anything is sent, where an SFTP upload lands, and the
//! bookkeeping of which sessions are uploading. The protocols themselves run
//! in the core's session loops; SFTP uploads run one file at a time on a
//! connection kept per terminal.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use omnyssh_core::event::SessionId;
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::sftp::{SftpCommand, SftpManager};
use omnyssh_core::transfer::{human_size, UploadResult, UploadState};

use crate::events::UploadProgress;

/// Typed into the shell to start a ZMODEM upload: `-b` for binary. Not `-e`:
/// the ZMODEM sender cannot escape every control character.
pub const RZ_COMMAND: &[u8] = b"rz -b\r";

/// The longest file name ZMODEM can carry, and the limit used for YMODEM too.
const MAX_NAME: usize = 256;

/// ZMODEM's file offsets are 32 bits.
const ZMODEM_MAX_SIZE: u64 = u32::MAX as u64;

/// The least time between two progress reports of a file, like the core's
/// protocol uploads.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// How a drop is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Zmodem,
    Ymodem,
    Xmodem,
    Sftp,
}

/// What the checks need to know about a dropped path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// Looks the dropped paths up on disk.
pub fn inspect(paths: &[String]) -> Result<Vec<Dropped>, String> {
    paths
        .iter()
        .map(|path| {
            let meta = std::fs::metadata(path).map_err(|e| format!("cannot read {path}: {e}"))?;
            let name = Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            if !meta.is_file() && !meta.is_dir() {
                return Err(format!("{name} is not a regular file"));
            }
            Ok(Dropped {
                path: path.clone(),
                name,
                size: meta.len(),
                is_dir: meta.is_dir(),
            })
        })
        .collect()
}

/// Whether `dropped` can go by `method`. Run before anything is sent, so a
/// refused drop never starts `rz` or reaches the device.
pub fn check(method: Method, dropped: &[Dropped]) -> Result<(), String> {
    if dropped.is_empty() {
        return Err("nothing to upload".to_string());
    }
    if let Some(dir) = dropped.iter().find(|d| d.is_dir) {
        return Err(format!("folders cannot be uploaded: {}", dir.name));
    }
    if method == Method::Xmodem && dropped.len() > 1 {
        return Err("XMODEM sends one file at a time".to_string());
    }
    for d in dropped {
        if method == Method::Zmodem && d.size > ZMODEM_MAX_SIZE {
            return Err(format!("{} is larger than ZMODEM's 4 GiB limit", d.name));
        }
        if matches!(method, Method::Zmodem | Method::Ymodem) && d.name.len() > MAX_NAME {
            return Err(format!(
                "{} has a name longer than {MAX_NAME} bytes",
                d.name
            ));
        }
    }
    Ok(())
}

/// Where an SFTP upload of `name` lands. `dir` is the host's upload folder:
/// unset or `~` is the home folder, `~/x` a folder in it, anything else as
/// written. SFTP resolves relative paths against the home folder, so `~`
/// becomes a relative path rather than being expanded here.
pub fn sftp_target(dir: Option<&str>, name: &str) -> String {
    let dir = dir.map(str::trim).unwrap_or("");
    let dir = match dir {
        "~" => "",
        d => d.strip_prefix("~/").unwrap_or(d),
    };
    match dir.trim_end_matches('/') {
        "" if dir.starts_with('/') => format!("/{name}"),
        "" => name.to_string(),
        dir => format!("{dir}/{name}"),
    }
}

/// The progress event for `state`; `None` for a final state.
pub fn progress_event(session_id: SessionId, state: &UploadState) -> Option<UploadProgress> {
    match state {
        UploadState::Waiting => Some(UploadProgress {
            session_id,
            waiting: true,
            file_index: 0,
            file_count: 0,
            file_name: String::new(),
            sent: 0,
            total: 0,
        }),
        UploadState::Sending {
            file_index,
            file_count,
            file_name,
            sent,
            total,
        } => Some(UploadProgress {
            session_id,
            waiting: false,
            file_index: *file_index as u32,
            file_count: *file_count as u32,
            file_name: file_name.clone(),
            sent: *sent,
            total: *total,
        }),
        _ => None,
    }
}

/// One file of an SFTP upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SftpFile {
    pub local: String,
    pub remote: String,
    pub name: String,
    pub size: u64,
}

/// An SFTP upload: files go one at a time, the next once the last is done, so
/// a failure or a cancel stops the rest.
#[derive(Debug)]
pub struct SftpUpload {
    files: Vec<SftpFile>,
    /// The file being sent.
    current: usize,
    cancelled: bool,
    /// When progress was last reported, and for which file.
    last_progress: Option<Instant>,
    progress_file: usize,
}

impl SftpUpload {
    pub fn new(files: Vec<SftpFile>) -> Self {
        Self {
            files,
            current: 0,
            cancelled: false,
            last_progress: None,
            progress_file: 0,
        }
    }

    fn command(&self) -> SftpCommand {
        let file = &self.files[self.current];
        SftpCommand::Upload {
            local: file.local.clone(),
            remote: file.remote.clone(),
            transfer_id: self.current as u64,
        }
    }

    /// The progress to report at `now`: none if the file's last report was less
    /// than `PROGRESS_INTERVAL` ago. The first report of each file always goes.
    fn progress(
        &mut self,
        session_id: SessionId,
        done: u64,
        total: u64,
        now: Instant,
    ) -> Option<UploadProgress> {
        if let Some(last) = self.last_progress {
            if self.progress_file == self.current && now.duration_since(last) < PROGRESS_INTERVAL {
                return None;
            }
        }
        self.last_progress = Some(now);
        self.progress_file = self.current;
        Some(UploadProgress {
            session_id,
            waiting: false,
            file_index: self.current as u32,
            file_count: self.files.len() as u32,
            file_name: self.files[self.current].name.clone(),
            sent: done,
            total,
        })
    }

    /// The current file is done: the next command to send, or the end.
    fn done(&mut self, result: Result<(), String>) -> Result<SftpCommand, UploadResult> {
        if let Err(e) = result {
            return Err(UploadResult {
                ok: false,
                message: format!("{}: {e}", self.files[self.current].name),
            });
        }
        if self.cancelled && self.current + 1 < self.files.len() {
            return Err(UploadResult {
                ok: false,
                message: "cancelled".to_string(),
            });
        }
        self.current += 1;
        if self.current < self.files.len() {
            return Ok(self.command());
        }
        let message = match self.files.as_slice() {
            [file] => format!("uploaded {} ({})", file.name, human_size(file.size)),
            files => {
                let total = files.iter().map(|f| f.size).sum();
                format!("uploaded {} files ({})", files.len(), human_size(total))
            }
        };
        Err(UploadResult { ok: true, message })
    }
}

/// A terminal's SFTP connection for uploads, and the upload on it, if any.
struct SftpConnection {
    manager: SftpManager,
    upload: Option<SftpUpload>,
}

/// Which sessions are uploading, and the per-terminal state uploads need.
/// Every method takes the public session id.
#[derive(Default)]
pub struct Uploads {
    /// Sessions with an upload running, of any kind: one at a time each.
    running: Mutex<HashSet<SessionId>>,
    /// Sessions whose SFTP upload was cancelled before it could start.
    cancel_requested: Mutex<HashSet<SessionId>>,
    /// The host each open terminal connected to, for SFTP uploads.
    hosts: Mutex<HashMap<SessionId, Host>>,
    /// SFTP connections, opened on a terminal's first SFTP drop and closed with it.
    sftp: Mutex<HashMap<SessionId, SftpConnection>>,
}

impl Uploads {
    /// Marks `id` uploading; refused if it already is.
    pub fn begin(&self, id: SessionId) -> Result<(), String> {
        if self
            .running
            .lock()
            .expect("uploads lock poisoned")
            .insert(id)
        {
            self.clear_cancel(id);
            Ok(())
        } else {
            Err("an upload is already running here".to_string())
        }
    }

    pub fn end(&self, id: SessionId) {
        self.running
            .lock()
            .expect("uploads lock poisoned")
            .remove(&id);
        self.clear_cancel(id);
    }

    fn clear_cancel(&self, id: SessionId) {
        self.cancel_requested
            .lock()
            .expect("uploads lock poisoned")
            .remove(&id);
    }

    /// Whether `id`'s upload was cancelled before it started sending.
    fn cancel_pending(&self, id: SessionId) -> bool {
        self.cancel_requested
            .lock()
            .expect("uploads lock poisoned")
            .contains(&id)
    }

    /// Records the host terminal `id` connected to: SFTP uploads go there, even
    /// if the host is edited or renamed meanwhile.
    pub fn terminal_opened(&self, id: SessionId, host: &Host) {
        self.hosts
            .lock()
            .expect("uploads lock poisoned")
            .insert(id, host.clone());
    }

    pub fn terminal_host(&self, id: SessionId) -> Option<Host> {
        self.hosts
            .lock()
            .expect("uploads lock poisoned")
            .get(&id)
            .cloned()
    }

    /// Session `id` ended: forget it, and close its SFTP connection.
    pub fn session_closed(&self, id: SessionId) {
        self.end(id);
        self.hosts
            .lock()
            .expect("uploads lock poisoned")
            .remove(&id);
        let conn = self.sftp.lock().expect("uploads lock poisoned").remove(&id);
        if let Some(conn) = conn {
            conn.manager.disconnect();
        }
    }

    pub fn has_sftp(&self, id: SessionId) -> bool {
        self.sftp
            .lock()
            .expect("uploads lock poisoned")
            .contains_key(&id)
    }

    /// Keeps `manager` for terminal `id`. Hands it back when the terminal
    /// closed while it was connecting.
    pub fn add_sftp(&self, id: SessionId, manager: SftpManager) -> Result<(), SftpManager> {
        // Lock order: `sftp`, then `hosts`. Holding `sftp` across the check
        // means a close cannot slip in between and leak the connection:
        // `session_closed` removes the host first, then takes `sftp`.
        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
        let open = self
            .hosts
            .lock()
            .expect("uploads lock poisoned")
            .contains_key(&id);
        if !open {
            return Err(manager);
        }
        sftp.insert(
            id,
            SftpConnection {
                manager,
                upload: None,
            },
        );
        Ok(())
    }

    /// Starts `upload` on terminal `id`'s SFTP connection.
    pub fn start_sftp(&self, id: SessionId, upload: SftpUpload) -> Result<(), String> {
        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
        if self.cancel_pending(id) {
            return Err("cancelled".to_string());
        }
        let conn = sftp.get_mut(&id).ok_or("the terminal is closed")?;
        conn.manager.send(upload.command());
        conn.upload = Some(upload);
        Ok(())
    }

    /// Stops terminal `id`'s SFTP upload after the file being sent. One that has
    /// not started yet (still connecting) is stopped before its first file.
    pub fn cancel_sftp(&self, id: SessionId) {
        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
        if let Some(upload) = sftp.get_mut(&id).and_then(|c| c.upload.as_mut()) {
            upload.cancelled = true;
        } else if self
            .running
            .lock()
            .expect("uploads lock poisoned")
            .contains(&id)
        {
            self.cancel_requested
                .lock()
                .expect("uploads lock poisoned")
                .insert(id);
        }
    }

    /// A progress report from terminal `id`'s SFTP connection.
    /// At most one per `PROGRESS_INTERVAL` for each file.
    pub fn sftp_progress(
        &self,
        id: SessionId,
        done: u64,
        total: u64,
        now: Instant,
    ) -> Option<UploadProgress> {
        let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
        let upload = sftp.get_mut(&id)?.upload.as_mut()?;
        upload.progress(id, done, total, now)
    }

    /// A file on terminal `id`'s SFTP connection is done: sends the next one,
    /// or returns how the upload ended (and marks `id` no longer uploading).
    pub fn sftp_done(&self, id: SessionId, result: Result<(), String>) -> Option<UploadResult> {
        let failed = result.is_err();
        let (end, dead) = {
            let mut sftp = self.sftp.lock().expect("uploads lock poisoned");
            let conn = sftp.get_mut(&id)?;
            let upload = conn.upload.as_mut()?;
            match upload.done(result) {
                Ok(next) => {
                    conn.manager.send(next);
                    return None;
                }
                Err(end) => {
                    conn.upload = None;
                    // A failed file may mean the link behind the connection
                    // died, and the core keeps the task until disconnect:
                    // drop it so the next drop reconnects.
                    let dead = if failed { sftp.remove(&id) } else { None };
                    (end, dead)
                }
            }
        };
        if let Some(conn) = dead {
            conn.manager.disconnect();
        }
        self.end(id);
        Some(end)
    }

    /// Terminal `id`'s SFTP connection is gone: forget it, and end the upload
    /// on it, if one was running.
    pub fn sftp_lost(&self, id: SessionId) -> Option<UploadResult> {
        let conn = self
            .sftp
            .lock()
            .expect("uploads lock poisoned")
            .remove(&id)?;
        conn.upload?;
        self.end(id);
        Some(UploadResult {
            ok: false,
            message: "the SFTP connection was lost".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64) -> Dropped {
        Dropped {
            path: format!("/tmp/{name}"),
            name: name.to_string(),
            size,
            is_dir: false,
        }
    }

    #[test]
    fn a_drop_is_checked_before_anything_is_sent() {
        let one = [file("a.bin", 10)];
        let two = [file("a.bin", 10), file("b.bin", 10)];
        for method in [Method::Zmodem, Method::Ymodem, Method::Xmodem, Method::Sftp] {
            assert_eq!(check(method, &one), Ok(()));
            assert_eq!(check(method, &[]), Err("nothing to upload".to_string()));
        }
        assert_eq!(check(Method::Ymodem, &two), Ok(()));
        assert_eq!(
            check(Method::Xmodem, &two),
            Err("XMODEM sends one file at a time".to_string())
        );
        let mut dir = file("photos", 0);
        dir.is_dir = true;
        assert_eq!(
            check(Method::Sftp, &[file("a", 1), dir]),
            Err("folders cannot be uploaded: photos".to_string())
        );
    }

    #[test]
    fn zmodem_limits_size_and_zmodem_and_ymodem_limit_names() {
        let big = [file("disk.img", 5 << 30)];
        assert_eq!(
            check(Method::Zmodem, &big),
            Err("disk.img is larger than ZMODEM's 4 GiB limit".to_string())
        );
        assert_eq!(check(Method::Sftp, &big), Ok(()));
        let long = [file(&"x".repeat(257), 1)];
        assert!(check(Method::Zmodem, &long).is_err());
        assert!(check(Method::Ymodem, &long).is_err());
        assert_eq!(check(Method::Xmodem, &long), Ok(()), "XMODEM sends no name");
    }

    #[test]
    fn inspect_reads_names_sizes_and_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, [0u8; 5]).unwrap();
        let paths = [
            path.to_string_lossy().into_owned(),
            dir.path().to_string_lossy().into_owned(),
        ];
        let found = inspect(&paths).unwrap();
        assert_eq!((found[0].name.as_str(), found[0].size), ("a.bin", 5));
        assert!(!found[0].is_dir);
        assert!(found[1].is_dir);
        let missing = dir.path().join("gone").to_string_lossy().into_owned();
        assert!(inspect(&[missing]).unwrap_err().starts_with("cannot read"));
    }

    #[test]
    fn sftp_targets_resolve_against_the_home_folder() {
        assert_eq!(sftp_target(None, "a.bin"), "a.bin");
        assert_eq!(sftp_target(Some("~"), "a.bin"), "a.bin");
        assert_eq!(sftp_target(Some(" ~/ "), "a.bin"), "a.bin");
        assert_eq!(sftp_target(Some("~/drop/"), "a.bin"), "drop/a.bin");
        assert_eq!(sftp_target(Some("/srv/in"), "a.bin"), "/srv/in/a.bin");
        assert_eq!(sftp_target(Some("/"), "a.bin"), "/a.bin");
        assert_eq!(sftp_target(Some("uploads"), "a.bin"), "uploads/a.bin");
    }

    #[test]
    fn progress_events_mirror_the_state() {
        let waiting = progress_event(7, &UploadState::Waiting).unwrap();
        assert!(waiting.waiting);
        let sending = progress_event(
            7,
            &UploadState::Sending {
                file_index: 1,
                file_count: 3,
                file_name: "b".to_string(),
                sent: 10,
                total: 20,
            },
        )
        .unwrap();
        assert_eq!(
            (sending.file_index, sending.file_count, sending.sent),
            (1, 3, 10)
        );
        assert!(progress_event(7, &UploadState::Cancelled).is_none());
    }

    fn sftp_files() -> Vec<SftpFile> {
        ["a", "b"]
            .iter()
            .map(|n| SftpFile {
                local: format!("/l/{n}"),
                remote: n.to_string(),
                name: n.to_string(),
                size: 1024,
            })
            .collect()
    }

    /// Where an upload command sends the file.
    fn remote(step: Result<SftpCommand, UploadResult>) -> String {
        match step {
            Ok(SftpCommand::Upload { remote, .. }) => remote,
            _ => panic!("expected the next upload"),
        }
    }

    fn ended(step: Result<SftpCommand, UploadResult>) -> UploadResult {
        match step {
            Err(end) => end,
            Ok(_) => panic!("expected the end"),
        }
    }

    #[test]
    fn sftp_files_go_one_after_another() {
        let mut up = SftpUpload::new(sftp_files());
        assert_eq!(remote(Ok(up.command())), "a");
        assert_eq!(
            up.progress(4, 5, 1024, Instant::now()).unwrap().file_name,
            "a"
        );
        assert_eq!(remote(up.done(Ok(()))), "b");
        assert_eq!(
            up.progress(4, 5, 1024, Instant::now()).unwrap().file_index,
            1
        );
        assert_eq!(
            ended(up.done(Ok(()))),
            UploadResult {
                ok: true,
                message: "uploaded 2 files (2.0 KB)".to_string()
            }
        );
    }

    #[test]
    fn an_sftp_failure_or_cancel_stops_the_rest() {
        let mut up = SftpUpload::new(sftp_files());
        let end = ended(up.done(Err("permission denied".to_string())));
        assert_eq!(end.message, "a: permission denied");
        assert!(!end.ok);

        let mut up = SftpUpload::new(sftp_files());
        up.cancelled = true;
        assert_eq!(ended(up.done(Ok(()))).message, "cancelled");
    }

    #[test]
    fn a_cancel_after_the_last_file_still_reports_success() {
        let mut up = SftpUpload::new(sftp_files());
        up.cancelled = true;
        assert!(up.done(Ok(())).is_err() && up.current == 0);
        let mut up = SftpUpload::new(sftp_files());
        let last = up.files.len() - 1;
        up.current = last;
        up.cancelled = true;
        let end = ended(up.done(Ok(())));
        assert!(end.ok, "{}", end.message);
        assert_ne!(end.message, "cancelled");
    }

    #[test]
    fn check_limits_are_inclusive() {
        let max = u64::from(u32::MAX);
        assert_eq!(check(Method::Zmodem, &[file("a", max)]), Ok(()));
        assert!(check(Method::Zmodem, &[file("a", max + 1)]).is_err());
        assert_eq!(check(Method::Zmodem, &[file(&"n".repeat(256), 1)]), Ok(()));
        assert!(check(Method::Zmodem, &[file(&"n".repeat(257), 1)]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn inspect_refuses_special_files() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("pipe");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let err = inspect(&[fifo.to_string_lossy().into_owned()]).unwrap_err();
        assert_eq!(err, "pipe is not a regular file");
    }

    #[test]
    fn one_upload_per_session() {
        let uploads = Uploads::default();
        assert_eq!(uploads.begin(1), Ok(()));
        assert_eq!(
            uploads.begin(1),
            Err("an upload is already running here".to_string())
        );
        assert_eq!(uploads.begin(2), Ok(()));
        uploads.end(1);
        assert_eq!(uploads.begin(1), Ok(()));
        uploads.session_closed(1);
        assert_eq!(uploads.begin(1), Ok(()), "a closed session is free again");
    }

    #[test]
    fn sftp_progress_is_throttled_per_file() {
        let mut up = SftpUpload::new(sftp_files());
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        assert!(up.progress(1, 10, 100, ms(0)).is_some(), "first goes");
        assert!(up.progress(1, 20, 100, ms(50)).is_none());
        assert!(up.progress(1, 30, 100, ms(99)).is_none());
        assert!(up.progress(1, 40, 100, ms(100)).is_some());
        assert!(up.progress(1, 50, 100, ms(150)).is_none());
        up.done(Ok(())).expect("next file");
        let next = up.progress(1, 5, 100, ms(160)).expect("a new file's first");
        assert_eq!(next.file_index, 1);
    }

    #[test]
    fn a_cancel_before_the_upload_starts_sticks() {
        let uploads = Uploads::default();
        uploads.cancel_sftp(1);
        assert!(
            !uploads.cancel_pending(1),
            "nothing running, nothing to cancel"
        );
        uploads.begin(1).expect("begin");
        uploads.cancel_sftp(1);
        assert!(uploads.cancel_pending(1));
        uploads.end(1);
        assert!(!uploads.cancel_pending(1));
        uploads.begin(1).expect("begin");
        uploads.cancel_sftp(1);
        uploads.session_closed(1);
        uploads.begin(1).expect("begin");
        assert!(!uploads.cancel_pending(1), "a new upload starts clean");
    }

    #[test]
    fn a_terminal_forgets_its_host_when_it_closes() {
        let uploads = Uploads::default();
        let web = Host {
            name: "web".to_string(),
            ..Host::default()
        };
        uploads.terminal_opened(3, &web);
        assert_eq!(uploads.terminal_host(3).map(|h| h.name), Some(web.name));
        uploads.session_closed(3);
        assert!(uploads.terminal_host(3).is_none());
        assert!(uploads.sftp_lost(3).is_none());
    }
}
