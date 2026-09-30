//! Serial port commands. Received bytes stream out on the tab's own raw
//! `Channel`, like a terminal's; a port that fails reports once on the tab's exit
//! channel, so nothing goes through the global event bus and no exit can race
//! ahead of the id the frontend is waiting for.

use omnyssh_core::config::serial_devices::{
    load_serial_devices, save_serial_devices, SerialDevice,
};
use omnyssh_core::serial::{self, SerialOutput, SerialSession};
use tauri::ipc::Channel;
use tauri::State;

use crate::dto::{SerialConfigDto, SerialDeviceDto, SerialExitDto, SerialPortDto, TerminalBytes};
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

/// The saved serial devices, for the dashboard's cards.
#[tauri::command]
#[specta::specta]
pub async fn list_serial_devices() -> Result<Vec<SerialDeviceDto>, CommandError> {
    let devices = tauri::async_runtime::spawn_blocking(load_serial_devices)
        .await
        .map_err(|e| CommandError {
            message: format!("serial device load task failed: {e}"),
        })?
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })?;
    Ok(devices.iter().map(SerialDeviceDto::from).collect())
}

/// Add a device, or replace the one with the same name.
#[tauri::command]
#[specta::specta]
pub async fn save_serial_device(device: SerialDeviceDto) -> Result<(), CommandError> {
    let device = SerialDevice::from(device);
    persist_devices(move |devices| upsert_device(devices, device)).await
}

/// Delete the device named `name`; a missing name is a no-op success.
#[tauri::command]
#[specta::specta]
pub async fn delete_serial_device(name: String) -> Result<(), CommandError> {
    persist_devices(move |devices| {
        devices.retain(|d| d.name != name);
        Ok(())
    })
    .await
}

/// Check `device` and add it, or replace the one with its name.
fn upsert_device(devices: &mut Vec<SerialDevice>, device: SerialDevice) -> Result<(), String> {
    if device.name.is_empty() {
        return Err("a serial device needs a name".to_string());
    }
    if device.port.trim().is_empty() {
        return Err("a serial device needs a port".to_string());
    }
    if device.baud_rate == 0 {
        return Err("the baud rate must be above 0".to_string());
    }
    if !(5..=8).contains(&device.data_bits) {
        return Err("data bits must be 5 to 8".to_string());
    }
    match devices.iter_mut().find(|d| d.name == device.name) {
        Some(slot) => *slot = device,
        None => devices.push(device),
    }
    Ok(())
}

/// Load the list, apply `mutate`, and write it back off the async worker.
async fn persist_devices(
    mutate: impl FnOnce(&mut Vec<SerialDevice>) -> Result<(), String> + Send + 'static,
) -> Result<(), CommandError> {
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let mut devices = load_serial_devices().map_err(|e| format!("{e:#}"))?;
        mutate(&mut devices)?;
        save_serial_devices(&devices).map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| CommandError {
        message: format!("serial device save task failed: {e}"),
    })?
    .map_err(|message| CommandError { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnyssh_core::config::serial_devices::LineEnding;
    use omnyssh_core::serial::{FlowControl, Parity, StopBits};

    fn device(name: &str, port: &str) -> SerialDevice {
        SerialDevice {
            name: name.to_string(),
            port: port.to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: StopBits::One,
            flow_control: FlowControl::None,
            enter: LineEnding::Cr,
            notes: None,
        }
    }

    #[test]
    fn saving_a_new_name_appends_and_an_existing_one_replaces() {
        let mut devices = vec![device("a", "COM1")];
        upsert_device(&mut devices, device("b", "COM2")).unwrap();
        upsert_device(&mut devices, device("a", "COM9")).unwrap();
        assert_eq!(devices, vec![device("a", "COM9"), device("b", "COM2")]);
    }

    #[test]
    fn a_device_needs_a_name_a_port_a_baud_rate_and_5_to_8_data_bits() {
        let mut devices = Vec::new();
        assert_eq!(
            upsert_device(&mut devices, device("", "COM1")).unwrap_err(),
            "a serial device needs a name"
        );
        assert_eq!(
            upsert_device(&mut devices, device("a", "")).unwrap_err(),
            "a serial device needs a port"
        );
        let mut slow = device("a", "COM1");
        slow.baud_rate = 0;
        assert_eq!(
            upsert_device(&mut devices, slow).unwrap_err(),
            "the baud rate must be above 0"
        );
        let mut wide = device("a", "COM1");
        wide.data_bits = 9;
        assert_eq!(
            upsert_device(&mut devices, wide).unwrap_err(),
            "data bits must be 5 to 8"
        );
        assert!(devices.is_empty());
    }

    #[test]
    fn the_dto_trims_the_name_and_drops_blank_notes() {
        let mut dto = SerialDeviceDto::from(&device("a", "COM1"));
        dto.name = "  board  ".to_string();
        dto.notes = Some("   ".to_string());
        let saved = SerialDevice::from(dto);
        assert_eq!(saved.name, "board");
        assert_eq!(saved.notes, None);
    }
}
