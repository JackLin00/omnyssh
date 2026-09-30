//! Serial port commands. Received bytes stream out on the tab's own raw
//! `Channel`, like a terminal's; a port that fails reports once on the tab's exit
//! channel, so nothing goes through the global event bus and no exit can race
//! ahead of the id the frontend is waiting for.

use omnyssh_core::serial::{self, SerialOutput, SerialSession};
use tauri::ipc::Channel;
use tauri::State;

use crate::dto::{SerialConfigDto, SerialExitDto, SerialPortDto, TerminalBytes};
use crate::error::CommandError;
use crate::state::GuiState;

/// The serial ports present right now. Async: enumeration can take 100ms+ on
/// Windows, so it runs off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn serial_list_ports() -> Result<Vec<SerialPortDto>, CommandError> {
    tauri::async_runtime::spawn_blocking(serial::list_ports)
        .await
        .map_err(|e| CommandError {
            message: format!("serial port listing task panicked: {e}"),
        })?
        .map(|ports| ports.into_iter().map(SerialPortDto::from).collect())
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}

/// Open a serial port and return the public session id the write/close commands
/// take. A missing or busy port fails here. The open itself runs in a blocking task —
/// it can stall for seconds (e.g. a Bluetooth SPP port) — so it never holds a tokio
/// worker thread.
#[tauri::command]
#[specta::specta]
pub async fn serial_open(
    state: State<'_, GuiState>,
    config: SerialConfigDto,
    on_output: Channel<TerminalBytes>,
    on_exit: Channel<SerialExitDto>,
) -> Result<u64, CommandError> {
    let id = state.allocate_session();
    let config = config.into();
    let session = tauri::async_runtime::spawn_blocking(move || {
        SerialSession::open(&config, move |out| match out {
            SerialOutput::Data(bytes) => {
                let _ = on_output.send(TerminalBytes(bytes));
            }
            SerialOutput::Closed(Some(error)) => {
                let _ = on_exit.send(SerialExitDto { error });
            }
            // Closed by the user: the tab is already gone.
            SerialOutput::Closed(None) => {}
        })
    })
    .await
    .map_err(|e| CommandError {
        message: format!("serial open task panicked: {e}"),
    })?
    .map_err(|e| CommandError {
        message: format!("{e:#}"),
    })?;
    state.insert_serial(id, session);
    Ok(id)
}

/// Send keystrokes to a serial port.
#[tauri::command]
#[specta::specta]
pub fn serial_write(
    state: State<'_, GuiState>,
    session_id: u64,
    data: Vec<u8>,
) -> Result<(), CommandError> {
    state.write_serial(session_id, &data);
    Ok(())
}

/// Close a serial port. Idempotent. Async: dropping the taken session closes the
/// port and joins its session thread (up to ~540ms with pending bytes on a stalled
/// line), so that wait runs in a blocking task instead of on the main thread. Returns
/// only once the port is released, so a reopen right after can succeed at once.
#[tauri::command]
#[specta::specta]
pub async fn serial_close(state: State<'_, GuiState>, session_id: u64) -> Result<(), CommandError> {
    let session = state.take_serial(session_id);
    tauri::async_runtime::spawn_blocking(move || drop(session))
        .await
        .map_err(|e| CommandError {
            message: format!("serial close task panicked: {e}"),
        })
}
