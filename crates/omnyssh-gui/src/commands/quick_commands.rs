//! Quick command groups for the terminal and serial tabs' command bar, saved whole to
//! `quick_commands.toml`. The frontend edits the groups and saves the lot; the save is
//! checked here, so a bad file is never written.

use omnyssh_core::config::quick_commands::{
    load_quick_commands, save_quick_commands as save_groups, PayloadKind, QuickGroup,
};

use crate::dto::QuickGroupDto;
use crate::error::CommandError;

/// Every group with its commands.
#[tauri::command]
#[specta::specta]
pub async fn list_quick_commands() -> Result<Vec<QuickGroupDto>, CommandError> {
    let groups = tauri::async_runtime::spawn_blocking(load_quick_commands)
        .await
        .map_err(|e| CommandError {
            message: format!("quick command load task failed: {e}"),
        })?
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })?;
    Ok(groups.iter().map(QuickGroupDto::from).collect())
}

/// Replace every group with `groups`, after checking them.
#[tauri::command]
#[specta::specta]
pub async fn save_quick_commands(groups: Vec<QuickGroupDto>) -> Result<(), CommandError> {
    let groups: Vec<QuickGroup> = groups.into_iter().map(QuickGroup::from).collect();
    validate_groups(&groups).map_err(|message| CommandError { message })?;
    tauri::async_runtime::spawn_blocking(move || save_groups(&groups))
        .await
        .map_err(|e| CommandError {
            message: format!("quick command save task failed: {e}"),
        })?
        .map_err(|e| CommandError {
            message: format!("{e:#}"),
        })
}

/// Names present and distinct, labels present, hex payloads whole bytes.
fn validate_groups(groups: &[QuickGroup]) -> Result<(), String> {
    for (i, g) in groups.iter().enumerate() {
        if g.name.is_empty() {
            return Err("a group needs a name".to_string());
        }
        if groups[..i].iter().any(|other| other.name == g.name) {
            return Err(format!("two groups are named '{}'", g.name));
        }
        for c in &g.commands {
            if c.label.is_empty() {
                return Err(format!("a command in '{}' needs a label", g.name));
            }
            if c.kind == PayloadKind::Hex && !is_hex_bytes(&c.payload) {
                return Err(format!("'{}' in '{}' is not hex bytes", c.label, g.name));
            }
        }
    }
    Ok(())
}

/// At least one byte of hex digit pairs, whitespace anywhere.
fn is_hex_bytes(s: &str) -> bool {
    let digits: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    !digits.is_empty()
        && digits.len().is_multiple_of(2)
        && digits.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnyssh_core::config::quick_commands::{Ending, PayloadKind, QuickCommand, QuickGroup};

    fn cmd(label: &str, kind: PayloadKind, payload: &str) -> QuickCommand {
        QuickCommand {
            label: label.to_string(),
            kind,
            payload: payload.to_string(),
            ending: Ending::Cr,
        }
    }

    fn group(name: &str, commands: Vec<QuickCommand>) -> QuickGroup {
        QuickGroup {
            name: name.to_string(),
            commands,
        }
    }

    #[test]
    fn valid_groups_pass() {
        let groups = vec![
            group(
                "Linux",
                vec![
                    cmd("disk", PayloadKind::Text, "df -h"),
                    cmd("enter", PayloadKind::Text, ""),
                ],
            ),
            group("Board", vec![cmd("ping", PayloadKind::Hex, "55 aa01")]),
        ];
        assert!(validate_groups(&groups).is_ok());
    }

    #[test]
    fn group_names_must_be_present_and_distinct() {
        assert_eq!(
            validate_groups(&[group("", vec![])]).unwrap_err(),
            "a group needs a name"
        );
        assert_eq!(
            validate_groups(&[group("A", vec![]), group("A", vec![])]).unwrap_err(),
            "two groups are named 'A'"
        );
    }

    #[test]
    fn commands_need_a_label_and_hex_must_be_whole_bytes() {
        assert_eq!(
            validate_groups(&[group("A", vec![cmd("", PayloadKind::Text, "x")])]).unwrap_err(),
            "a command in 'A' needs a label"
        );
        for bad in ["", "5", "5G", "55 A"] {
            assert_eq!(
                validate_groups(&[group("A", vec![cmd("h", PayloadKind::Hex, bad)])]).unwrap_err(),
                "'h' in 'A' is not hex bytes"
            );
        }
    }
}
