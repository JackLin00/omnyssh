//! Uploads to the real lrzsz receivers (`rz`, `rb`, `rx`) over pipes.
//!
//! Ignored by default: they need lrzsz. Run with
//! `cargo test -p omnyssh-core --test transfer_lrzsz -- --ignored`; set
//! `LRZSZ_DIR` to the directory holding the binaries when they are not on
//! `PATH`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use omnyssh_core::transfer::{start, Protocol, UploadFile, UploadState};

fn bin(name: &str) -> PathBuf {
    match std::env::var_os("LRZSZ_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
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
    let mut child = Command::new(bin(receiver))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("could not start {receiver} (is lrzsz installed?): {e}"));
    let mut to_rx = child.stdin.take().unwrap();
    let mut from_rx = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = from_rx.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let files = paths.iter().map(|p| UploadFile::open(p).unwrap()).collect();
    let mut up = start(protocol, files, Instant::now());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        assert!(Instant::now() < deadline, "timed out in {:?}", up.state());
        let mut out = Vec::new();
        up.transmit(&mut out);
        if !out.is_empty() {
            to_rx.write_all(&out).unwrap();
            to_rx.flush().unwrap();
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
    let status = child.wait().unwrap();
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
