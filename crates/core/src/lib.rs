//! Shared core for on-start-up: process management, persistence and
//! OS auto-start registration. No async runtime, no UI — the server daemon and
//! the Tauri shell both build on top of this.

pub mod alias_cmd;
pub mod autostart;
pub mod manager;
pub mod model;
pub mod trigger;

pub use manager::Manager;
pub use model::{Alerts, Alias, AliasCommand, CustomSound, Entry, ErrorPolicy, Event, LogLine, Settings, Trigger};

/// Bundle identifier; also the folder name inside the OS config directory.
pub const IDENTIFIER: &str = "com.seremtitus.onstartup";
/// Port the background server listens on (localhost only).
pub const DEFAULT_PORT: u16 = 46231;

/// Sink that receives every log line and status change produced by a [`Manager`].
pub type Sink = Box<dyn Fn(Event) + Send + Sync>;

/// File name of the background server binary next to this build.
pub const SERVER_BIN: &str = if cfg!(windows) {
    "on-start-up-server.exe"
} else {
    "on-start-up-server"
};

/// File name of the UI shell binary next to this build.
pub const UI_BIN: &str = if cfg!(windows) {
    "on-start-up.exe"
} else {
    "on-start-up"
};

/// File name of the error popup binary next to this build.
pub const ERROR_POPUP_BIN: &str = if cfg!(windows) {
    "on-start-up-error-popup.exe"
} else {
    "on-start-up-error-popup"
};

/// The exe that should be registered at login: the UI shell when it sits next
/// to this binary — it opens the window and makes sure the service is running
/// (closing the window leaves the service alone) — otherwise this binary.
pub fn launcher_exe() -> std::path::PathBuf {
    let self_exe = std::env::current_exe().unwrap_or_default();
    if let Some(dir) = self_exe.parent() {
        let ui = dir.join(UI_BIN);
        if ui.is_file() {
            return ui;
        }
    }
    self_exe
}

/// Config directory: `<config_dir>/com.seremtitus.onstartup`.
pub fn config_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(IDENTIFIER)
}

/// Server port, overridable via `ON_START_UP_PORT`.
pub fn port() -> u16 {
    std::env::var("ON_START_UP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}
