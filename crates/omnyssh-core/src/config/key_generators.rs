//! Key generators: JS code that turns a shared credential and a device's MAC
//! and revision into that device's SSH private key, in
//! `~/.config/omnyssh/key_generators.toml`. Off unless `enabled`. The credential
//! holds a master key, so on Unix the file is written owner-only from the first
//! byte; elsewhere it takes the folder's permissions, as `hosts.toml` does.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::utils::platform;

/// One generator: the code defining `generateKey(deviceInfo, address)`, and the
/// credential (JSON text) every device using it shares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyGenerator {
    pub name: String,
    #[serde(default)]
    pub code: String,
    /// JSON text; empty for none.
    #[serde(default)]
    pub credential: String,
}

/// The whole file: the switch and the generators.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyGenerators {
    /// Nothing is generated while this is off, whatever the hosts say.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub generators: Vec<KeyGenerator>,
}

impl KeyGenerators {
    /// The generator named `name`.
    pub fn find(&self, name: &str) -> Option<&KeyGenerator> {
        self.generators.iter().find(|g| g.name == name)
    }
}

/// Loads the key generators. A missing file is switched off, with none.
///
/// # Errors
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_key_generators() -> anyhow::Result<KeyGenerators> {
    let path =
        platform::key_generators_config_path().context("Cannot determine key generators path")?;
    load_from(&path)
}

/// Persists the key generators.
///
/// # Errors
/// Returns an error if the directory cannot be created or the file cannot be written.
pub fn save_key_generators(config: &KeyGenerators) -> anyhow::Result<()> {
    let path =
        platform::key_generators_config_path().context("Cannot determine key generators path")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {}", dir.display()))?;
    }
    save_to(&path, config)
}

fn load_from(path: &Path) -> anyhow::Result<KeyGenerators> {
    if !path.exists() {
        return Ok(KeyGenerators::default());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))
}

fn save_to(path: &Path, config: &KeyGenerators) -> anyhow::Result<()> {
    let content = toml::to_string_pretty(config).context("Failed to serialise key generators")?;
    // A temp file renamed over the old one, so an interrupted write never leaves a
    // corrupt file; on Unix created owner-only, so the credential is never readable
    // by others.
    let tmp_path = path.with_extension("toml.tmp");
    write_private(&tmp_path, content.as_bytes())
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
    Ok(())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    // A temp file left by an earlier crash keeps its old mode: set it again.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> KeyGenerators {
        KeyGenerators {
            enabled: true,
            generators: vec![KeyGenerator {
                name: "vendor-a".to_string(),
                code: "async function generateKey(d, a) {\n  return {};\n}\n".to_string(),
                credential: r#"{"master_key":"00ff"}"#.to_string(),
            }],
        }
    }

    #[test]
    fn saved_generators_read_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key_generators.toml");
        save_to(&path, &config()).unwrap();
        assert_eq!(load_from(&path).unwrap(), config());
    }

    #[test]
    fn a_missing_file_is_switched_off_with_none() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_from(&dir.path().join("key_generators.toml")).unwrap();
        assert_eq!(loaded, KeyGenerators::default());
        assert!(!loaded.enabled);
    }

    #[test]
    fn omitted_fields_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key_generators.toml");
        std::fs::write(&path, "[[generators]]\nname = \"g\"\n").unwrap();
        let loaded = load_from(&path).unwrap();
        assert!(!loaded.enabled);
        assert_eq!(loaded.find("g").unwrap().code, "");
        assert_eq!(loaded.find("g").unwrap().credential, "");
        assert!(loaded.find("other").is_none());
    }

    #[test]
    fn a_malformed_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key_generators.toml");
        std::fs::write(&path, "generators = 1").unwrap();
        assert!(format!("{:#}", load_from(&path).unwrap_err()).contains("key_generators.toml"));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key_generators.toml");
        // A stale temp file from a crash, readable by everyone.
        std::fs::write(path.with_extension("toml.tmp"), "").unwrap();
        std::fs::set_permissions(
            path.with_extension("toml.tmp"),
            std::fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        save_to(&path, &config()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
