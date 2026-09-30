//! Saved serial devices, shown as cards on the desktop dashboard, in
//! `~/.config/omnyssh/serial.toml`. Written the way `snippets.toml` is: whole-file,
//! through a temp file and a rename, owner-only on Unix.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::serial::{FlowControl, Parity, SerialConfig, StopBits};
use crate::utils::platform;

/// What the Enter key sends in a serial terminal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineEnding {
    #[default]
    Cr,
    Lf,
    Crlf,
}

fn default_data_bits() -> u8 {
    8
}

/// One saved device: a name, its port and line settings, and what Enter sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialDevice {
    pub name: String,
    pub port: String,
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    #[serde(default)]
    pub parity: Parity,
    #[serde(default)]
    pub stop_bits: StopBits,
    #[serde(default)]
    pub flow_control: FlowControl,
    #[serde(default)]
    pub enter: LineEnding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl SerialDevice {
    /// The line settings to open the port with.
    pub fn config(&self) -> SerialConfig {
        SerialConfig {
            port: self.port.clone(),
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            parity: self.parity,
            stop_bits: self.stop_bits,
            flow_control: self.flow_control,
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SerialDevicesFile {
    #[serde(default)]
    devices: Vec<SerialDevice>,
}

/// Loads the saved devices. A missing file is an empty list.
///
/// # Errors
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_serial_devices() -> anyhow::Result<Vec<SerialDevice>> {
    let path =
        platform::serial_devices_config_path().context("Cannot determine serial devices path")?;
    load_from(&path)
}

/// Persists the whole device list.
///
/// # Errors
/// Returns an error if the directory cannot be created or the file cannot be written.
pub fn save_serial_devices(devices: &[SerialDevice]) -> anyhow::Result<()> {
    let path =
        platform::serial_devices_config_path().context("Cannot determine serial devices path")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {}", dir.display()))?;
    }
    save_to(&path, devices)
}

fn load_from(path: &Path) -> anyhow::Result<Vec<SerialDevice>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let file: SerialDevicesFile =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(file.devices)
}

fn save_to(path: &Path, devices: &[SerialDevice]) -> anyhow::Result<()> {
    let file = SerialDevicesFile {
        devices: devices.to_vec(),
    };
    let content = toml::to_string_pretty(&file).context("Failed to serialise serial devices")?;
    // Write to a temp file and rename, so an interrupted write never leaves a corrupt file.
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &content)
        .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e).with_context(|| {
            format!(
                "Failed to rename {} to {}",
                tmp_path.display(),
                path.display()
            )
        });
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str) -> SerialDevice {
        SerialDevice {
            name: name.to_string(),
            port: "COM3".to_string(),
            baud_rate: 921_600,
            data_bits: 7,
            parity: Parity::Even,
            stop_bits: StopBits::Two,
            flow_control: FlowControl::Hardware,
            enter: LineEnding::Crlf,
            notes: Some("bench".to_string()),
        }
    }

    #[test]
    fn a_saved_list_reads_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serial.toml");
        let devices = vec![device("esp32"), device("stm32")];
        save_to(&path, &devices).unwrap();
        assert_eq!(load_from(&path).unwrap(), devices);
    }

    #[test]
    fn a_missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from(&dir.path().join("serial.toml"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn omitted_line_settings_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serial.toml");
        std::fs::write(
            &path,
            "[[devices]]\nname = \"board\"\nport = \"COM4\"\nbaud_rate = 115200\n",
        )
        .unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(
            loaded,
            vec![SerialDevice {
                name: "board".to_string(),
                port: "COM4".to_string(),
                baud_rate: 115_200,
                data_bits: 8,
                parity: Parity::None,
                stop_bits: StopBits::One,
                flow_control: FlowControl::None,
                enter: LineEnding::Cr,
                notes: None,
            }]
        );
    }

    #[test]
    fn a_malformed_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serial.toml");
        std::fs::write(&path, "devices = 3").unwrap();
        let err = load_from(&path).unwrap_err();
        assert!(format!("{err:#}").contains("serial.toml"));
    }
}
