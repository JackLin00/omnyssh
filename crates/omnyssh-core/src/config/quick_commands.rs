//! Quick commands for the terminal and serial tabs' command bar, in groups, in
//! `~/.config/omnyssh/quick_commands.toml`. Written the way `serial.toml` is: whole-file,
//! through a temp file and a rename, owner-only on Unix.

use std::path::Path;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::utils::platform;

/// How a command's payload is read: as text, or as hex bytes like `55 AA 01`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PayloadKind {
    #[default]
    Text,
    Hex,
}

/// What is sent after the payload. CR is what a terminal's Enter sends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ending {
    None,
    #[default]
    Cr,
    Lf,
    Crlf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickCommand {
    pub label: String,
    #[serde(default)]
    pub kind: PayloadKind,
    #[serde(default)]
    pub payload: String,
    #[serde(default)]
    pub ending: Ending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickGroup {
    pub name: String,
    #[serde(default)]
    pub commands: Vec<QuickCommand>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct QuickCommandsFile {
    #[serde(default)]
    groups: Vec<QuickGroup>,
}

/// Loads the saved quick command groups. A missing file is no groups.
///
/// # Errors
/// Returns an error if the file exists but cannot be read or parsed.
pub fn load_quick_commands() -> anyhow::Result<Vec<QuickGroup>> {
    let path =
        platform::quick_commands_config_path().context("Cannot determine quick commands path")?;
    load_from(&path)
}

/// Persists the whole group list.
///
/// # Errors
/// Returns an error if the directory cannot be created or the file cannot be written.
pub fn save_quick_commands(groups: &[QuickGroup]) -> anyhow::Result<()> {
    let path =
        platform::quick_commands_config_path().context("Cannot determine quick commands path")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory {}", dir.display()))?;
    }
    save_to(&path, groups)
}

fn load_from(path: &Path) -> anyhow::Result<Vec<QuickGroup>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let file: QuickCommandsFile =
        toml::from_str(&content).with_context(|| format!("Failed to parse {}", path.display()))?;
    Ok(file.groups)
}

fn save_to(path: &Path, groups: &[QuickGroup]) -> anyhow::Result<()> {
    let file = QuickCommandsFile {
        groups: groups.to_vec(),
    };
    let content = toml::to_string_pretty(&file).context("Failed to serialise quick commands")?;
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

    fn groups() -> Vec<QuickGroup> {
        vec![
            QuickGroup {
                name: "Linux".to_string(),
                commands: vec![QuickCommand {
                    label: "disk".to_string(),
                    kind: PayloadKind::Text,
                    payload: "df -h".to_string(),
                    ending: Ending::Cr,
                }],
            },
            QuickGroup {
                name: "Board".to_string(),
                commands: vec![QuickCommand {
                    label: "ping".to_string(),
                    kind: PayloadKind::Hex,
                    payload: "55 AA 01".to_string(),
                    ending: Ending::None,
                }],
            },
        ]
    }

    #[test]
    fn saved_groups_read_back_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quick_commands.toml");
        save_to(&path, &groups()).unwrap();
        assert_eq!(load_from(&path).unwrap(), groups());
    }

    #[test]
    fn a_missing_file_is_no_groups() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from(&dir.path().join("quick_commands.toml"))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn omitted_fields_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quick_commands.toml");
        std::fs::write(
            &path,
            "[[groups]]\nname = \"g\"\n[[groups.commands]]\nlabel = \"ls\"\npayload = \"ls\"\n",
        )
        .unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded[0].commands[0].kind, PayloadKind::Text);
        assert_eq!(loaded[0].commands[0].ending, Ending::Cr);
    }

    #[test]
    fn a_malformed_file_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quick_commands.toml");
        std::fs::write(&path, "groups = 1").unwrap();
        assert!(format!("{:#}", load_from(&path).unwrap_err()).contains("quick_commands.toml"));
    }
}
