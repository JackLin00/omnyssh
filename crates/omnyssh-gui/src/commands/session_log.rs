//! Session logs and exports (plan N): the save dialog, starting and stopping a
//! log, writing an export, and showing a saved file in its folder. Only a path the
//! user picked in the save dialog is ever written or shown, so none of these is a
//! general file channel for the webview.

use omnyssh_core::session_log::{HexLogger, SessionLogger, TextLogger, NATIVE_NEWLINE};
use tauri::{Manager, State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;

use crate::dto::{LogModeDto, LogStatusDto, SaveKindDto};
use crate::error::CommandError;
use crate::state::GuiState;

fn failed(message: impl Into<String>) -> CommandError {
    CommandError {
        message: message.into(),
    }
}

/// The dialog's title and file types. Windows adds the first type's extension to
/// a name typed without one.
fn dialog_style(kind: SaveKindDto) -> (&'static str, [(&'static str, &'static str); 2]) {
    let (log, txt) = (("Log files", "log"), ("Text files", "txt"));
    match kind {
        SaveKindDto::Log => ("Save session log", [log, txt]),
        SaveKindDto::Export => ("Export terminal contents", [txt, log]),
    }
}

/// Ask where to save, starting from `default_name` in the folder of the last
/// save (the Documents folder the first time). `None` when the user cancels.
/// Overwriting an existing file is confirmed by the dialog itself.
#[tauri::command]
#[specta::specta]
pub async fn pick_save_path(
    window: WebviewWindow,
    state: State<'_, GuiState>,
    default_name: String,
    kind: SaveKindDto,
) -> Result<Option<String>, CommandError> {
    let (title, filters) = dialog_style(kind);
    let mut dialog = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title(title)
        .set_file_name(default_name);
    for (name, extension) in filters {
        dialog = dialog.add_filter(name, &[extension]);
    }
    if let Some(dir) = state
        .save_dir()
        .or_else(|| window.path().document_dir().ok())
    {
        dialog = dialog.set_directory(dir);
    }
    // The callback form: the dialog runs on the main thread, and this command
    // waits without holding a runtime worker.
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.save_file(move |picked| {
        let _ = tx.send(picked);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| failed(e.to_string()))?;
    Ok(Some(state.remember_save_path(path)))
}

/// Start logging session `session_id` into `path` (from `pick_save_path`), with
/// `header` as the first line. `timestamps` stamps each text line; a hex log is
/// never stamped.
#[tauri::command]
#[specta::specta]
pub async fn log_start(
    state: State<'_, GuiState>,
    session_id: u64,
    path: String,
    timestamps: bool,
    mode: LogModeDto,
    header: String,
) -> Result<(), CommandError> {
    let logger = match mode {
        LogModeDto::Text => SessionLogger::Text(TextLogger::new(timestamps, NATIVE_NEWLINE)),
        LogModeDto::Hex => SessionLogger::Hex(HexLogger::new(NATIVE_NEWLINE)),
    };
    state
        .start_log(session_id, &path, logger, &header)
        .map_err(failed)
}

/// Stop logging a session; `log-stopped` follows. A no-op when it is not logged.
#[tauri::command]
#[specta::specta]
pub async fn log_stop(state: State<'_, GuiState>, session_id: u64) -> Result<(), CommandError> {
    state.logs().stop(session_id);
    Ok(())
}

/// The file a session is logged into and the bytes written so far, if it is.
#[tauri::command]
#[specta::specta]
pub async fn log_status(
    state: State<'_, GuiState>,
    session_id: u64,
) -> Result<Option<LogStatusDto>, CommandError> {
    Ok(state
        .logs()
        .status(session_id)
        .map(|(path, bytes)| LogStatusDto { path, bytes }))
}

/// Write an export (`\n`-separated lines) to `path`, from `pick_save_path`, with
/// this platform's line ends.
#[tauri::command]
#[specta::specta]
pub async fn save_text_file(
    state: State<'_, GuiState>,
    path: String,
    text: String,
) -> Result<(), CommandError> {
    let path = state.picked_path(&path).map_err(failed)?;
    // A log open on this same path would otherwise be truncated out from under its
    // writer by this export.
    if state.logs().is_logging_path(&path) {
        return Err(failed("That file is being logged to"));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let text = text.replace('\n', NATIVE_NEWLINE);
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    })
    .await
    .map_err(|e| failed(format!("export task panicked: {e}")))?
    .map_err(failed)
}

/// Show a saved log or export, selected, in the system file manager.
#[tauri::command]
#[specta::specta]
pub async fn reveal_path(state: State<'_, GuiState>, path: String) -> Result<(), CommandError> {
    let path = state.picked_path(&path).map_err(failed)?;
    tauri::async_runtime::spawn_blocking(move || tauri_plugin_opener::reveal_item_in_dir(path))
        .await
        .map_err(|e| failed(format!("reveal task panicked: {e}")))?
        .map_err(|e| failed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_saves_as_log_and_an_export_as_txt_by_default() {
        assert_eq!(dialog_style(SaveKindDto::Log).1[0], ("Log files", "log"));
        assert_eq!(
            dialog_style(SaveKindDto::Export).1[0],
            ("Text files", "txt")
        );
    }
}
