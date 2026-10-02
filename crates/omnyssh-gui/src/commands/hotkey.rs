//! The global show/hide hotkey (plan J): one system-wide shortcut, registered
//! through `tauri-plugin-global-shortcut`, that brings the window forward or hides
//! it again. The frontend owns the preference and hands it over through
//! `set_global_hotkey`; this app never registers anything on its own.

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::error::CommandError;

/// What the global hotkey does to the main window.
#[derive(Debug, PartialEq, Eq)]
enum Toggle {
    Show,
    /// Hide into the tray when it can hold the window, else minimize.
    Hide {
        to_tray: bool,
    },
}

/// A window the user is looking at hides; any other comes forward.
fn toggle(visible: bool, minimized: bool, focused: bool, tray_hides: bool) -> Toggle {
    if visible && !minimized && focused {
        Toggle::Hide {
            to_tray: tray_hides,
        }
    } else {
        Toggle::Show
    }
}

/// Called from the plugin's handler on every `Pressed` event (never `Released`,
/// which would otherwise toggle twice per key press), already dispatched onto the
/// main thread (see the handler in `main.rs` for why).
pub fn on_hotkey(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let minimized = window.is_minimized().unwrap_or(false);
    // On X11 a key grab can land while the window manager still reports some other
    // window focused — observed, not yet reliably reproduced — which could make this
    // always read as unfocused and the hotkey always Show there. Untested; left as a
    // follow-up rather than guessed at here.
    let focused = window.is_focused().unwrap_or(false);
    match toggle(visible, minimized, focused, crate::tray::hides_to_tray()) {
        Toggle::Show => crate::tray::reveal(app),
        Toggle::Hide { to_tray: true } => {
            let _ = window.hide();
        }
        Toggle::Hide { to_tray: false } => {
            let _ = window.minimize();
        }
    }
}

/// Register `accelerator` (e.g. "Control+Space") as the global show/hide hotkey,
/// replacing the previous one; None turns it off. A hotkey another program holds, or
/// one the OS rejects, comes back as the error.
#[tauri::command]
#[specta::specta]
pub fn set_global_hotkey(app: AppHandle, accelerator: Option<String>) -> Result<(), CommandError> {
    let shortcuts = app.global_shortcut();
    shortcuts.unregister_all().map_err(|e| CommandError {
        message: e.to_string(),
    })?;
    let Some(accelerator) = accelerator else {
        return Ok(()); // turning it off always succeeds, Wayland included
    };
    if crate::tray::is_wayland(&app) {
        return Err(CommandError {
            message: "Global shortcuts aren't available in Wayland sessions".into(),
        });
    }
    shortcuts
        .register(accelerator.as_str())
        .map_err(|e| registration_error(&accelerator, e))
}

/// `global-hotkey`'s own message for a key combo already held elsewhere names the raw
/// `HotKey` struct via its `Debug` form rather than anything a person reading Settings
/// would recognize; every other error keeps its own message.
fn registration_error(accelerator: &str, e: impl std::fmt::Display) -> CommandError {
    let message = e.to_string();
    if message.to_lowercase().contains("already registered") {
        CommandError {
            message: format!("{accelerator} is already used by another program"),
        }
    } else {
        CommandError { message }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use tauri_plugin_global_shortcut::Shortcut;

    use super::*;

    #[test]
    fn a_focused_window_hides_and_any_other_shows() {
        assert_eq!(
            toggle(true, false, true, true),
            Toggle::Hide { to_tray: true }
        );
        assert_eq!(
            toggle(true, false, true, false),
            Toggle::Hide { to_tray: false }
        );
        assert_eq!(toggle(true, false, false, true), Toggle::Show); // behind other windows
        assert_eq!(toggle(true, true, false, true), Toggle::Show); // minimized
        assert_eq!(toggle(false, false, false, true), Toggle::Show); // in the tray
    }

    /// The accelerator strings the frontend's `chordToAccelerator` (ui/src/lib/stores/
    /// globalHotkey.ts) produces must parse the way this crate's own `Shortcut::from_str`
    /// (re-exported `global_hotkey::hotkey::HotKey::from_str`) actually accepts them —
    /// checked here against the real parser rather than assumed.
    #[test]
    fn accelerator_strings_the_frontend_produces_parse() {
        for accelerator in ["Control+Space", "Super+K", "Control+Alt+Backquote"] {
            assert!(
                Shortcut::from_str(accelerator).is_ok(),
                "{accelerator} should parse"
            );
        }
        // A code the parser has no entry for (unlike the handful terminalShortcuts.ts
        // can produce) must be rejected, not silently registered as something else.
        assert!(Shortcut::from_str("Control+ContextMenu").is_err());
    }

    #[test]
    fn an_already_registered_error_reads_as_the_accelerator_in_use() {
        // The real message, `global_hotkey::Error::AlreadyRegistered`'s Display, names
        // the raw `HotKey` struct via `{:?}` rather than anything readable.
        let raw = "HotKey already registered: HotKey { mods: CONTROL, key: Space, id: 123 }";
        assert_eq!(
            registration_error("Control+Space", raw).message,
            "Control+Space is already used by another program"
        );
        assert_eq!(
            registration_error("Control+Space", "some other failure").message,
            "some other failure"
        );
    }
}
