//! Drag-and-drop upload commands. Each checks the drop first (folders, XMODEM's
//! one file, ZMODEM's limits, an upload already running), so a refused drop
//! sends nothing. Progress and the end arrive as `upload-progress` and
//! `upload-finished`. After a command returns `Ok`, the upload ends with one
//! `upload-finished`, or, if its session ends first, with no upload event at
//! all: the frontend also treats `terminal-exited`, the serial tab's exit
//! channel and closing the tab as the end. (An SFTP upload whose terminal's
//! shell exits remotely, and one queued while a terminal was still connecting
//! whose connection then failed, get no `upload-finished`.)

use std::time::Instant;

use omnyssh_core::ssh::host_key::HostKeyPrompter;
use omnyssh_core::ssh::identity;
use omnyssh_core::ssh::password::Prompter;
use omnyssh_core::ssh::sftp::SftpManager;
use omnyssh_core::transfer::{ActiveUpload, Protocol, UploadFile};
use tauri::{AppHandle, State};
use tokio::sync::mpsc;

use crate::bridge;
use crate::dto::{HostUploadProtocolDto, SerialUploadProtocolDto};
use crate::error::CommandError;
use crate::state::GuiState;
use crate::upload::{self, Dropped, Method, SftpFile, SftpUpload, RZ_COMMAND};

/// The upload connection's event buffer, like an SFTP tab's.
const SFTP_EVENT_BUFFER: usize = 256;

/// Upload `paths` into terminal `session_id`: ZMODEM to `rz` in its shell, or
/// SFTP into the host's upload folder over a connection kept for the terminal.
/// A cancel that arrives while the SFTP connection is still being made makes this
/// return `Err("cancelled")`: treat that as a cancel, not a failure.
#[tauri::command]
#[specta::specta]
pub async fn terminal_upload(
    app: AppHandle,
    state: State<'_, GuiState>,
    session_id: u64,
    protocol: HostUploadProtocolDto,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let method = match protocol {
        HostUploadProtocolDto::Zmodem => Method::Zmodem,
        HostUploadProtocolDto::Sftp => Method::Sftp,
    };
    let dropped = checked(method, &paths)?;
    state.uploads().begin(session_id).map_err(error)?;
    let started = match method {
        Method::Sftp => start_sftp(&app, &state, session_id, &dropped).await,
        _ => open(&dropped).and_then(|files| {
            let upload = ActiveUpload::start(Protocol::Zmodem, files, Instant::now());
            state.upload_terminal(session_id, RZ_COMMAND, upload)
        }),
    };
    if started.is_err() {
        state.uploads().end(session_id);
    }
    started.map_err(error)
}

/// Upload `paths` to serial port `session_id`. ZMODEM types `rz -b` first;
/// YMODEM and XMODEM wait for the receiver the user started (`loady`, `rx`).
#[tauri::command]
#[specta::specta]
pub async fn serial_upload(
    state: State<'_, GuiState>,
    session_id: u64,
    protocol: SerialUploadProtocolDto,
    paths: Vec<String>,
) -> Result<(), CommandError> {
    let (method, protocol, preamble): (_, _, &[u8]) = match protocol {
        SerialUploadProtocolDto::Zmodem => (Method::Zmodem, Protocol::Zmodem, RZ_COMMAND),
        SerialUploadProtocolDto::Ymodem => (Method::Ymodem, Protocol::Ymodem, b""),
        SerialUploadProtocolDto::Xmodem => (Method::Xmodem, Protocol::Xmodem, b""),
    };
    let dropped = checked(method, &paths)?;
    state.uploads().begin(session_id).map_err(error)?;
    let started = open(&dropped).and_then(|files| {
        let upload = ActiveUpload::start(protocol, files, Instant::now());
        state.upload_serial(session_id, preamble, upload)
    });
    if started.is_err() {
        state.uploads().end(session_id);
    }
    started.map_err(error)
}

/// Cancel session `session_id`'s upload. ZMODEM, YMODEM and XMODEM stop at once;
/// SFTP after the file being sent. A session without one is a no-op. Cancelling
/// while the SFTP connection is still being made makes `terminal_upload` return
/// `Err("cancelled")`; treat that as a cancel, not a failure.
#[tauri::command]
#[specta::specta]
pub fn upload_cancel(state: State<'_, GuiState>, session_id: u64) -> Result<(), CommandError> {
    state.cancel_upload(session_id);
    Ok(())
}

fn checked(method: Method, paths: &[String]) -> Result<Vec<Dropped>, CommandError> {
    let dropped = upload::inspect(paths).map_err(error)?;
    upload::check(method, &dropped).map_err(error)?;
    Ok(dropped)
}

fn open(dropped: &[Dropped]) -> Result<Vec<UploadFile>, String> {
    dropped
        .iter()
        .map(|d| {
            UploadFile::open(std::path::Path::new(&d.path))
                .map_err(|e| format!("cannot read {}: {e}", d.name))
        })
        .collect()
}

/// Sends `dropped` over terminal `id`'s SFTP connection, opening it first.
async fn start_sftp(
    app: &AppHandle,
    state: &GuiState,
    id: u64,
    dropped: &[Dropped],
) -> Result<(), String> {
    // The host the terminal connected to, not whatever its name means now.
    let host = state
        .uploads()
        .terminal_host(id)
        .ok_or("the terminal is closed")?;
    if !state.uploads().has_sftp(id) {
        let (tx, rx) = mpsc::channel(SFTP_EVENT_BUFFER);
        let prompter = Prompter::new(state.engine_sender(), &host.name);
        let host_keys = HostKeyPrompter::new(state.engine_sender(), &host.name);
        let manager =
            match SftpManager::connect_asking_host_keys(&host, tx, prompter, host_keys).await {
                Ok(manager) => manager,
                Err(e) => {
                    if let Some(path) = omnyssh_core::ssh::session::passphrase_required(&e) {
                        identity::ask_passphrase(&state.engine_sender(), &host.name, path).await;
                    }
                    return Err(format!("{e:#}"));
                }
            };
        let generation = match state.uploads().add_sftp(id, manager) {
            Ok(generation) => generation,
            Err(manager) => {
                manager.disconnect();
                return Err("the terminal is closed".to_string());
            }
        };
        tauri::async_runtime::spawn(bridge::forward_upload_sftp_events(
            app.clone(),
            id,
            generation,
            rx,
        ));
    }
    let files = dropped
        .iter()
        .map(|d| SftpFile {
            local: d.path.clone(),
            remote: upload::sftp_target(host.upload_dir.as_deref(), &d.name),
            name: d.name.clone(),
            size: d.size,
        })
        .collect();
    state.uploads().start_sftp(id, SftpUpload::new(files))
}

fn error(message: impl Into<String>) -> CommandError {
    CommandError {
        message: message.into(),
    }
}
