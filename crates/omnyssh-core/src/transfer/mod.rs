//! Sending files to a device over a terminal byte stream: ZMODEM to `rz`,
//! YMODEM and XMODEM to bootloaders such as U-Boot's `loady`/`loadx`.
//!
//! Each protocol is a sans-IO state machine behind [`Upload`]: the session
//! that owns the port or channel feeds it what the device sends, takes what it
//! has to send, and drives its timers with the current time. Nothing here
//! touches a port, a channel or the clock, so the tests run the protocols
//! against in-memory receivers with a hand-advanced `Instant`.

use std::fs::File;
use std::io::{self, Read, Seek};
use std::path::Path;
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

mod xmodem;
mod zmodem;

/// How much one [`Upload::transmit`] call adds at most. The caller only asks
/// again once it has sent it all, so this bounds what sits in its buffer.
pub(crate) const TRANSMIT_CHUNK: usize = 8 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Zmodem,
    Ymodem,
    Xmodem,
}

impl Protocol {
    /// Whether one upload can carry several files.
    pub fn batch(self) -> bool {
        !matches!(self, Protocol::Xmodem)
    }
}

/// A file's contents, read and re-read (ZMODEM resends from an offset the
/// receiver asks for).
pub trait Source: Read + Seek + Send {}
impl<T: Read + Seek + Send> Source for T {}

pub struct UploadFile {
    /// The name the receiver saves it under: the base name, no directories.
    pub name: String,
    pub size: u64,
    /// Last change, in seconds since the Unix epoch.
    pub modified: Option<u64>,
    pub reader: Box<dyn Source>,
}

impl UploadFile {
    /// Opens a local regular file for upload.
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a file"));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Ok(Self {
            name,
            size: meta.len(),
            modified,
            reader: Box::new(file),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadState {
    /// Waiting for the receiver to start (ZRINIT, `C` or NAK).
    Waiting,
    Sending {
        /// 0-based.
        file_index: usize,
        file_count: usize,
        file_name: String,
        sent: u64,
        total: u64,
    },
    Done {
        sent_files: usize,
        skipped: usize,
    },
    Failed(String),
    Cancelled,
}

impl UploadState {
    /// Done, failed or cancelled: the session leaves upload mode once
    /// [`Upload::transmit`] has nothing more to send.
    pub fn is_final(&self) -> bool {
        matches!(
            self,
            UploadState::Done { .. } | UploadState::Failed(_) | UploadState::Cancelled
        )
    }
}

/// One upload in progress. Contract for the caller:
/// - pass every byte the device sends to `receive`;
/// - call `tick` regularly (every 100 ms or so) and after `receive`;
/// - call `transmit` whenever its own send buffer is empty, and send what it
///   adds;
/// - once `state()` is final and `transmit` adds nothing, hand `leftover()` to
///   the terminal and stop routing bytes here.
pub trait Upload: Send {
    fn receive(&mut self, data: &[u8]);
    fn transmit(&mut self, out: &mut Vec<u8>);
    fn tick(&mut self, now: Instant);
    /// Stops at once; the cancel sequence comes out of the next `transmit`.
    fn cancel(&mut self);
    fn state(&self) -> UploadState;
    /// What the device sent after the transfer ended (rz's trailing output,
    /// the shell prompt), to show in the terminal.
    fn leftover(&mut self) -> Vec<u8>;
}

/// Starts an upload. `files` must not be empty, and XMODEM takes exactly one
/// (see [`Protocol::batch`]); callers check both first.
pub fn start(protocol: Protocol, files: Vec<UploadFile>, now: Instant) -> Box<dyn Upload> {
    assert!(!files.is_empty(), "an upload needs a file");
    match protocol {
        Protocol::Zmodem => Box::new(zmodem::ZmodemUpload::new(files, now)),
        Protocol::Ymodem => Box::new(xmodem::XmodemUpload::new(
            xmodem::Variant::Ymodem,
            files,
            now,
        )),
        Protocol::Xmodem => {
            assert!(files.len() == 1, "XMODEM sends one file");
            Box::new(xmodem::XmodemUpload::new(
                xmodem::Variant::Xmodem,
                files,
                now,
            ))
        }
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use std::io::Cursor;

    pub fn file(name: &str, data: Vec<u8>) -> UploadFile {
        UploadFile {
            name: name.to_string(),
            size: data.len() as u64,
            modified: Some(1_700_000_000),
            reader: Box::new(Cursor::new(data)),
        }
    }

    /// Deterministic pseudo-random bytes.
    pub fn bytes(len: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761).max(1);
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect()
    }

    /// Takes everything the upload has to send right now.
    pub fn drain(up: &mut dyn Upload) -> Vec<u8> {
        let mut all = Vec::new();
        loop {
            let mut out = Vec::new();
            up.transmit(&mut out);
            if out.is_empty() {
                return all;
            }
            all.extend(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_reads_name_size_and_time() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fw.bin");
        std::fs::write(&path, b"12345").unwrap();
        let f = UploadFile::open(&path).unwrap();
        assert_eq!(f.name, "fw.bin");
        assert_eq!(f.size, 5);
        assert!(f.modified.is_some());
    }

    #[test]
    fn open_rejects_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(UploadFile::open(dir.path()).is_err());
    }

    #[test]
    fn only_xmodem_is_single_file() {
        assert!(Protocol::Zmodem.batch());
        assert!(Protocol::Ymodem.batch());
        assert!(!Protocol::Xmodem.batch());
    }
}
