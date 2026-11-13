//! Tauri UI shell. The window is only a *client*: all process management lives
//! in the detached background server (`on-start-up-server`), which keeps
//! running after this window closes. This binary makes sure the service is up
//! and points the frontend at it. The tray icon is owned by the server.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::Manager;



#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerInfo {
    url: String,
    port: u16,
    server_exe: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EnsureResult {
    running: bool,
    started: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Background service helpers
// ---------------------------------------------------------------------------

/// Locate the background server binary: next to this exe first (installed
/// layout), then the workspace target dirs (dev layout).
fn find_server_exe() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("ON_START_UP_SERVER") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let direct = dir.join(on_start_up_core::SERVER_BIN);
    if direct.is_file() {
        return Some(direct);
    }
    for rel in ["../debug", "../release"] {
        let candidate = dir.join(rel).join(on_start_up_core::SERVER_BIN);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok()
}

/// Launch the server fully detached â€” it must outlive this UI process.
fn spawn_server(exe: &PathBuf) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        Command::new(exe)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        Command::new(exe)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}



// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn server_info() -> ServerInfo {
    let port = on_start_up_core::port();
    ServerInfo {
        url: format!("http://127.0.0.1:{port}"),
        port,
        server_exe: find_server_exe().map(|p| p.display().to_string()),
    }
}

#[tauri::command]
fn ensure_server() -> EnsureResult {
    let port = on_start_up_core::port();
    if port_open(port) {
        return EnsureResult {
            running: true,
            started: false,
            error: None,
        };
    }

    let Some(exe) = find_server_exe() else {
        return EnsureResult {
            running: false,
            started: false,
            error: Some(format!(
                "background service binary '{}' not found next to the app",
                on_start_up_core::SERVER_BIN
            )),
        };
    };

    if let Err(e) = spawn_server(&exe) {
        return EnsureResult {
            running: false,
            started: false,
            error: Some(format!("failed to launch background service: {e}")),
        };
    }

    let deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < deadline {
        if port_open(port) {
            return EnsureResult {
                running: true,
                started: true,
                error: None,
            };
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    EnsureResult {
        running: false,
        started: false,
        error: Some("background service did not start in time".into()),
    }
}

#[tauri::command]
fn open_in_file_manager(path: String) -> Result<(), String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("directory path is empty".into());
    }

    #[cfg(target_os = "windows")]
    {
        Command::new("explorer")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("failed to open file manager: {e}"))?;
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("failed to open file manager: {e}"))?;
        return Ok(());
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("failed to open file manager: {e}"))?;
        Ok(())
    }
}


#[cfg(windows)]
fn detach_from_terminal_if_needed() {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    // In release builds, never stay linked to a terminal: relaunch detached so
    // closing the launching terminal never closes this UI.
    if cfg!(debug_assertions) {
        return;
    }

    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    if std::env::args().any(|a| a == "--detached-launch") {
        return;
    }

    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = Command::new(exe);
        cmd.args(std::env::args().skip(1));
        cmd.arg("--detached-launch");
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
        if cmd.spawn().is_ok() {
            std::process::exit(0);
        }
    }
}

#[cfg(not(windows))]
fn detach_from_terminal_if_needed() {}

fn main() {
    detach_from_terminal_if_needed();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            // Tray icon and error popups are owned/triggered by the background server.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            server_info,
            ensure_server,
            open_in_file_manager
        ])
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|_, _| {});
}
