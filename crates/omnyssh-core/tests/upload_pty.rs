//! Uploads through a terminal, end to end: `PtyManager` against an in-process SSH
//! server whose "shell" reads lines and, given `rz -b`, runs the real `rz` on
//! the channel — or answers `command not found` when there is none.

#![cfg(unix)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use russh::keys::key::KeyPair;
use russh::server::{self, Auth, Msg, Session};
use russh::{Channel, ChannelId, CryptoVec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use omnyssh_core::event::CoreEvent;
use omnyssh_core::ssh::client::Host;
use omnyssh_core::ssh::pty::PtyManager;
use omnyssh_core::transfer::{
    ActiveUpload, Protocol, UploadEvent, UploadFile, UploadResult, UploadState,
};

const PASSWORD: &str = "upload-test";

/// Keeps trust-on-first-use off the real `~/.ssh`, and the local agent out.
fn isolate_home() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let home = tempfile::tempdir().expect("tempdir").keep();
        std::env::set_var("HOME", &home);
        std::env::remove_var("SSH_AUTH_SOCK");
    });
}

/// `rz` from `LRZSZ_DIR`, or from `PATH`.
fn rz() -> PathBuf {
    std::env::var_os("LRZSZ_DIR")
        .map(|dir| PathBuf::from(dir).join("rz"))
        .unwrap_or_else(|| PathBuf::from("rz"))
}

// ---------------------------------------------------------------------------
// SSH server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Server {
    /// Where `rz` saves; `None` plays a shell without lrzsz.
    rz_dir: Option<PathBuf>,
    /// The line typed so far, while no `rz` runs.
    line: Arc<Mutex<Vec<u8>>>,
    /// Input for the running `rz`.
    to_rz: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    /// Every byte the channel received, in order.
    received: Arc<Mutex<Vec<u8>>>,
}

#[async_trait::async_trait]
impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == PASSWORD {
            Auth::Accept
        } else {
            Auth::Reject {
                proceed_with_methods: None,
            }
        })
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, CryptoVec::from_slice(b"$ "));
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.received.lock().unwrap().extend_from_slice(data);
        if let Some(to_rz) = self.to_rz.lock().unwrap().as_ref() {
            let _ = to_rz.send(data.to_vec());
            return Ok(());
        }
        for &byte in data {
            if byte != b'\r' {
                self.line.lock().unwrap().push(byte);
                continue;
            }
            let line = std::mem::take(&mut *self.line.lock().unwrap());
            match (&self.rz_dir, line.as_slice()) {
                (Some(dir), b"rz -b") => self.start_rz(dir, channel, session),
                (None, b"rz -b") => session.data(
                    channel,
                    CryptoVec::from_slice(b"rz -b\r\nsh: rz: command not found\r\n$ "),
                ),
                _ => session.data(channel, CryptoVec::from_slice(b"\r\n$ ")),
            }
        }
        Ok(())
    }
}

impl Server {
    /// Runs `rz -b` in `dir` with its stdin and stdout on the channel, then
    /// prints a prompt when it exits, the way a shell would.
    fn start_rz(&self, dir: &PathBuf, channel: ChannelId, session: &Session) {
        let mut child = tokio::process::Command::new(rz())
            .arg("-b")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("cannot run rz (is lrzsz installed? set LRZSZ_DIR)");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        *self.to_rz.lock().unwrap() = Some(tx);
        tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                if stdin.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        });
        let handle = session.handle();
        let to_rz = Arc::clone(&self.to_rz);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match stdout.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let _ = handle.data(channel, CryptoVec::from_slice(&buf[..n])).await;
                    }
                }
            }
            let _ = child.wait().await;
            *to_rz.lock().unwrap() = None;
            let _ = handle.data(channel, CryptoVec::from_slice(b"$ ")).await;
        });
    }
}

async fn serve(rz_dir: Option<PathBuf>) -> SocketAddr {
    serve_recording(rz_dir).await.0
}

/// Like [`serve`], also returning everything the channel receives.
async fn serve_recording(rz_dir: Option<PathBuf>) -> (SocketAddr, Arc<Mutex<Vec<u8>>>) {
    let server = Server {
        rz_dir,
        line: Arc::default(),
        to_rz: Arc::default(),
        received: Arc::default(),
    };
    let received = Arc::clone(&server.received);
    let config = Arc::new(server::Config {
        keys: vec![KeyPair::generate_ed25519()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let _ = server::run_stream(Arc::clone(&config), socket, server.clone()).await;
        }
    });
    (addr, received)
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

struct Terminal {
    pty: PtyManager,
    id: u64,
    events: mpsc::Receiver<CoreEvent>,
}

impl Terminal {
    async fn open(addr: SocketAddr) -> Self {
        let term = Self::connect(addr, Some(PASSWORD)).await;
        assert!(term.screen_contains("$ ").await, "no shell");
        term
    }

    /// Opens a terminal without waiting for the shell.
    async fn connect(addr: SocketAddr, password: Option<&str>) -> Self {
        isolate_home();
        let host = Host {
            name: "box".to_string(),
            hostname: addr.ip().to_string(),
            port: addr.port(),
            user: "me".to_string(),
            password: password.map(str::to_string),
            ..Host::default()
        };
        let (tx, events) = mpsc::channel::<CoreEvent>(1024);
        let mut pty = PtyManager::new();
        let id = pty.open(&host, 80, 24, tx).expect("open");
        Self { pty, id, events }
    }

    fn screen(&self) -> String {
        let parser = self.pty.parser_for(self.id).expect("session");
        let text = parser.lock().unwrap().screen().contents();
        text
    }

    async fn screen_contains(&self, text: &str) -> bool {
        for _ in 0..200 {
            if self.screen().contains(text) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    /// Upload events until the end, which is returned last.
    async fn until_finished(&mut self) -> (Vec<UploadState>, UploadResult) {
        let mut progress = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(30), self.events.recv())
                .await
                .expect("the upload never finished")
                .expect("session gone");
            match event {
                CoreEvent::Upload(id, UploadEvent::Progress(state)) if id == self.id => {
                    progress.push(state)
                }
                CoreEvent::Upload(id, UploadEvent::Finished(result)) if id == self.id => {
                    return (progress, result)
                }
                _ => {}
            }
        }
    }
}

fn file(name: &str, data: Vec<u8>) -> UploadFile {
    UploadFile {
        name: name.to_string(),
        size: data.len() as u64,
        modified: Some(1_700_000_000),
        reader: Box::new(std::io::Cursor::new(data)),
    }
}

/// Random-looking bytes with every value, so escaping is exercised.
fn bytes(len: usize) -> Vec<u8> {
    let mut x: u32 = 0x9e37_79b9;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "needs lrzsz"]
async fn a_zmodem_upload_through_a_terminal_reaches_rz() {
    let dir = tempfile::tempdir().unwrap();
    let addr = serve(Some(dir.path().to_path_buf())).await;
    let mut term = Terminal::open(addr).await;

    let data = bytes(300 * 1024);
    let upload = ActiveUpload::start(
        Protocol::Zmodem,
        vec![
            file("big.bin", data.clone()),
            file("small.txt", b"hi\n".to_vec()),
        ],
        Instant::now(),
    );
    term.pty.upload(term.id, b"rz -b\r", upload);

    let (progress, result) = term.until_finished().await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.message, "uploaded 2 files (300.0 KB)");
    assert!(progress
        .iter()
        .any(|s| matches!(s, UploadState::Sending { .. })));
    assert_eq!(std::fs::read(dir.path().join("big.bin")).unwrap(), data);
    assert_eq!(
        std::fs::read(dir.path().join("small.txt")).unwrap(),
        b"hi\n"
    );

    assert!(term.screen_contains("[upload] uploaded 2 files").await);
    // No protocol bytes on the screen: the frames start with `**` + ZDLE.
    assert!(!term.screen().contains("**"), "{}", term.screen());
    // The terminal is the shell's again: a typed line gets a new prompt.
    let prompts = term.screen().matches("$ ").count();
    term.pty.write(term.id, b"ls\r").unwrap();
    let mut back = false;
    for _ in 0..100 {
        if term.screen().matches("$ ").count() > prompts {
            back = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(back, "{}", term.screen());
}

#[tokio::test]
async fn cancelling_an_upload_the_shell_cannot_take_shows_its_reply() {
    let addr = serve(None).await;
    let mut term = Terminal::open(addr).await;

    let upload = ActiveUpload::start(
        Protocol::Zmodem,
        vec![file("a.bin", vec![1, 2, 3])],
        Instant::now(),
    );
    term.pty.upload(term.id, b"rz -b\r", upload);
    // The shell's answer is held back while the upload waits for rz.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!term.screen().contains("command not found"));

    term.pty.cancel_upload(term.id);
    let (_, result) = term.until_finished().await;
    assert_eq!(result.message, "cancelled");
    assert!(term.screen_contains("command not found").await);
    assert!(term.screen_contains("[upload] cancelled").await);
}

/// A terminal still at the inline password prompt, with an upload sent to it.
async fn terminal_at_the_password_prompt() -> Terminal {
    let addr = serve(None).await;
    let term = Terminal::connect(addr, None).await;
    assert!(term.screen_contains("password: ").await, "no prompt");
    term
}

fn an_upload() -> ActiveUpload {
    ActiveUpload::start(
        Protocol::Zmodem,
        vec![file("a.bin", vec![1, 2, 3])],
        Instant::now(),
    )
}

#[tokio::test]
async fn an_upload_sent_while_logging_in_is_refused() {
    let mut term = terminal_at_the_password_prompt().await;
    term.pty.upload(term.id, b"rz -b\r", an_upload());
    term.pty
        .write(term.id, format!("{PASSWORD}\r").as_bytes())
        .unwrap();

    let (_, result) = term.until_finished().await;
    assert!(!result.ok);
    assert_eq!(result.message, "the terminal was still logging in");
}

#[tokio::test]
async fn an_upload_sent_while_logging_in_is_refused_and_the_tab_closed() {
    let mut term = terminal_at_the_password_prompt().await;
    term.pty.upload(term.id, b"rz -b\r", an_upload());
    term.pty.close(term.id);

    // The end comes before the tab's exit.
    let mut finished = None;
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), term.events.recv())
            .await
            .expect("no PtyExited")
            .expect("session gone");
        match event {
            CoreEvent::Upload(id, UploadEvent::Finished(r)) if id == term.id => finished = Some(r),
            CoreEvent::PtyExited(id) if id == term.id => break,
            _ => {}
        }
    }
    let result = finished.expect("no Finished before PtyExited");
    assert!(!result.ok);
    assert_eq!(result.message, "the terminal was still logging in");
}

#[tokio::test]
async fn closing_a_terminal_mid_upload_sends_the_cancel_sequence() {
    let (addr, received) = serve_recording(None).await;
    let mut term = Terminal::open(addr).await;

    term.pty.upload(term.id, b"rz -b\r", an_upload());
    tokio::time::sleep(Duration::from_millis(300)).await;
    term.pty.close(term.id);

    let (_, result) = term.until_finished().await;
    assert_eq!(result.message, "cancelled");

    let cancel = [0x18u8; 10];
    let mut seen = false;
    for _ in 0..40 {
        let got = received.lock().unwrap().clone();
        if let Some(at) = got.windows(6).position(|w| w == b"rz -b\r") {
            if got[at + 6..].windows(10).any(|w| w == cancel) {
                seen = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        seen,
        "no cancel sequence after the preamble: {:?}",
        received.lock().unwrap()
    );
}
