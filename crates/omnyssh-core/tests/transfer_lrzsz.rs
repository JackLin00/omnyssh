//! Uploads to the real lrzsz receivers (`rz`, `rb`, `rx`) over pipes.
//!
//! Ignored by default: they need lrzsz. Run with
//! `cargo test -p omnyssh-core --test transfer_lrzsz -- --ignored`; set
//! `LRZSZ_DIR` to the directory holding the binaries when they are not on
//! `PATH`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use omnyssh_core::transfer::{start, Protocol, UploadFile, UploadState};

fn bin(name: &str) -> PathBuf {
    match std::env::var_os("LRZSZ_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

/// Kills and reaps the child on every exit path, including a panic.
struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Runs `receiver` in `dir` and uploads `paths` to it; returns the final state.
fn upload(
    receiver: &str,
    args: &[&str],
    dir: &Path,
    protocol: Protocol,
    paths: &[PathBuf],
) -> UploadState {
    let mut child = ChildGuard(
        Command::new(bin(receiver))
            .args(args)
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap_or_else(|e| panic!("could not start {receiver} (is lrzsz installed?): {e}")),
    );
    let pid = child.0.id();
    let mut to_rx = child.0.stdin.take().unwrap();
    let mut from_rx = child.0.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = from_rx.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    // `write_all` and `child.wait()` below block, so a stalled receiver would
    // hang forever instead of hitting the loop's own deadline assert. This
    // watchdog enforces the deadline in wall-clock time regardless of what
    // the main thread is blocked on, by killing the receiver (Unix only:
    // these tests only run where lrzsz exists, i.e. Unix).
    let deadline = Instant::now() + Duration::from_secs(60);
    let timed_out = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel::<()>();
    {
        let timed_out = Arc::clone(&timed_out);
        std::thread::spawn(move || {
            let wait = deadline.saturating_duration_since(Instant::now());
            if matches!(
                done_rx.recv_timeout(wait),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                timed_out.store(true, Ordering::SeqCst);
                let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
            }
        });
    }

    let files = paths.iter().map(|p| UploadFile::open(p).unwrap()).collect();
    let mut up = start(protocol, files, Instant::now());
    loop {
        assert!(Instant::now() < deadline, "timed out in {:?}", up.state());
        let mut out = Vec::new();
        up.transmit(&mut out);
        if !out.is_empty() {
            to_rx
                .write_all(&out)
                .expect("write to the receiver failed (killed by the 60 s watchdog?)");
            to_rx
                .flush()
                .expect("flush to the receiver failed (killed by the 60 s watchdog?)");
            continue;
        }
        if up.state().is_final() {
            break;
        }
        if let Ok(data) = rx.recv_timeout(Duration::from_millis(20)) {
            up.receive(&data);
        }
        up.tick(Instant::now());
    }
    drop(to_rx);
    let status = child
        .0
        .wait()
        .expect("waiting on the receiver failed (killed by the 60 s watchdog?)");
    drop(done_tx); // the upload finished; cancel the watchdog
    assert!(
        !timed_out.load(Ordering::SeqCst),
        "{receiver} was killed by the 60 s watchdog"
    );
    let state = up.state();
    if matches!(state, UploadState::Done { .. }) {
        assert!(status.success(), "{receiver} exited with {status}");
    }
    state
}

fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

#[test]
#[ignore = "needs lrzsz"]
fn zmodem_to_rz() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let big = random(1 << 20, 1);
    let paths = vec![
        write(src.path(), "big.bin", &big),
        write(src.path(), "small.txt", b"hello\n"),
        write(src.path(), "empty", b""),
    ];
    let state = upload("rz", &["-b"], dst.path(), Protocol::Zmodem, &paths);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 3,
            skipped: 0
        }
    );
    assert_eq!(std::fs::read(dst.path().join("big.bin")).unwrap(), big);
    assert_eq!(
        std::fs::read(dst.path().join("small.txt")).unwrap(),
        b"hello\n"
    );
    assert!(std::fs::read(dst.path().join("empty")).unwrap().is_empty());

    // Again: rz skips files that already exist.
    let state = upload("rz", &["-b"], dst.path(), Protocol::Zmodem, &paths[..2]);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 0,
            skipped: 2
        }
    );
}

#[test]
#[ignore = "needs lrzsz"]
fn ymodem_to_rb() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let big = random((1 << 20) + 77, 2);
    let paths = vec![
        write(src.path(), "fw.bin", &big),
        write(src.path(), "b.txt", b"hi"),
    ];
    let state = upload("rb", &["-b"], dst.path(), Protocol::Ymodem, &paths);
    assert_eq!(
        state,
        UploadState::Done {
            sent_files: 2,
            skipped: 0
        }
    );
    // YMODEM's block 0 carries the size, so the padding is cut off.
    assert_eq!(std::fs::read(dst.path().join("fw.bin")).unwrap(), big);
    assert_eq!(std::fs::read(dst.path().join("b.txt")).unwrap(), b"hi");
}

#[test]
#[ignore = "needs lrzsz"]
fn xmodem_to_rx_with_crc_and_with_checksum() {
    for args in [&["-b", "-c", "out.bin"][..], &["-b", "out.bin"][..]] {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        // A whole number of 128-byte blocks: XMODEM pads the last one.
        let data = random(128 * 1000, 3);
        let paths = vec![write(src.path(), "in.bin", &data)];
        let state = upload("rx", args, dst.path(), Protocol::Xmodem, &paths);
        assert_eq!(
            state,
            UploadState::Done {
                sent_files: 1,
                skipped: 0
            },
            "rx {args:?}"
        );
        assert_eq!(
            std::fs::read(dst.path().join("out.bin")).unwrap(),
            data,
            "rx {args:?}"
        );
    }
}
