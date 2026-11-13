//! Headless background server for on-start-up.
//!
//! Runs detached from any UI: it is started at login, launches every listed
//! process on start, and exposes a localhost HTTP API + SSE stream that any
//! number of UI windows can attach to. Closing the UI never touches the
//! processes.

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::{
    extract::State,
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_core::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

use on_start_up_core::model::{Alias, Entry};
use on_start_up_core::{autostart, config_dir, port, trigger, Event, Manager, Settings};

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

struct AppInner {
    manager: Arc<Manager>,
    events: broadcast::Sender<Event>,
}

type App = Arc<AppInner>;
type ApiError = (StatusCode, Json<Value>);
type ApiResult = Result<Json<Value>, ApiError>;

fn err(code: StatusCode, msg: &str) -> ApiError {
    (code, Json(json!({ "error": msg })))
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct IdBody {
    id: String,
}

#[derive(Deserialize)]
struct UpdateEntryBody {
    entry: Entry,
}

fn default_alias_command() -> String {
    "osu".to_string()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsBody {
    autostart: bool,
    #[serde(default)]
    alias_active: bool,
    #[serde(default = "default_alias_command")]
    alias_command: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn ping() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn get_state(State(app): State<App>) -> Json<Value> {
    Json(json!({
        "entries": app.manager.entries(),
        "aliases": app.manager.aliases(),
        "logs": app.manager.logs(),
        "running": app.manager.running(),
        "exits": app.manager.exits(),
        "settings": app.manager.settings(),
    }))
}

async fn add_entry(State(app): State<App>, Json(entry): Json<Entry>) -> Json<Value> {
    let entry = app.manager.add_entry(entry);
    Json(json!({ "ok": true, "entry": entry }))
}

async fn update_entry(State(app): State<App>, Json(body): Json<UpdateEntryBody>) -> ApiResult {
    app.manager
        .update_entry(body.entry)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_entry(State(app): State<App>, Json(body): Json<IdBody>) -> ApiResult {
    app.manager
        .remove_entry(&body.id)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn start(State(app): State<App>, Json(body): Json<IdBody>) -> ApiResult {
    app.manager
        .start(&body.id)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn stop(State(app): State<App>, Json(body): Json<IdBody>) -> ApiResult {
    app.manager
        .stop(&body.id)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn start_all(State(app): State<App>) -> Json<Value> {
    let errors = app.manager.start_all();
    Json(json!({ "ok": true, "errors": errors }))
}

async fn stop_all(State(app): State<App>) -> Json<Value> {
    app.manager.stop_all();
    Json(json!({ "ok": true }))
}

async fn clear_logs(State(app): State<App>, Json(body): Json<IdBody>) -> Json<Value> {
    app.manager.clear_logs(&body.id);
    Json(json!({ "ok": true }))
}

async fn add_alias(State(app): State<App>, Json(alias): Json<Alias>) -> ApiResult {
    let alias = app
        .manager
        .add_alias(alias)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true, "alias": alias })))
}

async fn update_alias(State(app): State<App>, Json(alias): Json<Alias>) -> ApiResult {
    app.manager
        .update_alias(alias)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_alias(State(app): State<App>, Json(body): Json<IdBody>) -> ApiResult {
    app.manager
        .remove_alias(&body.id)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;
    Ok(Json(json!({ "ok": true })))
}

async fn set_settings(State(app): State<App>, Json(body): Json<SettingsBody>) -> ApiResult {
    let alias_command = on_start_up_core::alias_cmd::normalize_command_name(&body.alias_command);
    on_start_up_core::alias_cmd::validate_command_name(&alias_command)
        .map_err(|e| err(StatusCode::BAD_REQUEST, &e))?;

    let exe = on_start_up_core::launcher_exe();
    let result = if body.autostart {
        autostart::install(&exe)
    } else {
        autostart::uninstall()
    };
    result.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    let old = app.manager.settings();
    let old_command = on_start_up_core::alias_cmd::normalize_command_name(&old.alias_command);

    let alias_result = if body.alias_active {
        let r = on_start_up_core::alias_cmd::install(&alias_command);
        if r.is_ok() && old_command != alias_command {
            let _ = on_start_up_core::alias_cmd::uninstall(&old_command);
        }
        r
    } else {
        // Deactivate current and also clean up previous name if different.
        let r = on_start_up_core::alias_cmd::uninstall(&alias_command);
        if old_command != alias_command {
            let _ = on_start_up_core::alias_cmd::uninstall(&old_command);
        }
        r
    };
    alias_result.map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e))?;

    app.manager.set_settings(Settings {
        autostart: body.autostart,
        alias_active: body.alias_active,
        alias_command,
    });
    Ok(Json(json!({ "ok": true })))
}

async fn shutdown(State(app): State<App>) -> Json<Value> {
    app.manager.stop_all();
    stop_tray();
    kill_ui_and_popups();
    tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(400)).await;
        std::process::exit(0);
    });
    Json(json!({ "ok": true }))
}

/// Server-sent events: every log line and status change, as it happens.
async fn events(
    State(app): State<App>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let mut rx = app.events.subscribe();
    let stream = async_stream::stream! {
        yield Ok(SseEvent::default().data(r#"{"type":"hello"}"#));
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if let Ok(data) = serde_json::to_string(&event) {
                        yield Ok(SseEvent::default().data(data));
                    }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    yield Ok(SseEvent::default().data(format!(r#"{{"type":"lagged","count":{count}}}"#)));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// ---------------------------------------------------------------------------
// Logging (the server has no console, so write to a file)
// ---------------------------------------------------------------------------

fn log_to(path: &Path, msg: &str) {
    use std::io::Write;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{ts}] {msg}");
    }
}

fn rotate_log(path: &Path) {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > 2 * 1024 * 1024 {
            let _ = std::fs::remove_file(path);
        }
    }
}

static TRAY_CHILD: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

fn tray_cell() -> &'static Mutex<Option<Child>> {
    TRAY_CHILD.get_or_init(|| Mutex::new(None))
}

fn find_ui_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let direct = dir.join(on_start_up_core::UI_BIN);
    if direct.is_file() {
        return Some(direct);
    }
    // dev layouts
    for rel in [".", "../debug", "../release", "../../debug", "../../release"] {
        let candidate = dir.join(rel).join(on_start_up_core::UI_BIN);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_icon_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("icon.ico"),
        dir.join("icons").join("icon.ico"),
        dir.join("../src-tauri/icons/icon.ico"),
        dir.join("../../src-tauri/icons/icon.ico"),
    ];
    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

fn find_error_popup_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let direct = dir.join(on_start_up_core::ERROR_POPUP_BIN);
    if direct.is_file() {
        return Some(direct);
    }
    for rel in [".", "../debug", "../release", "../../debug", "../../release"] {
        let candidate = dir.join(rel).join(on_start_up_core::ERROR_POPUP_BIN);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn spawn_error_popup(
    manager: &Manager,
    id: &str,
    exit_code: Option<i32>,
    log_path: &Path,
) {
    let Some(exe) = find_error_popup_exe() else {
        log_to(log_path, "error popup: executable not found");
        return;
    };
    let entry = manager.entries().into_iter().find(|e| e.id == id);
    let Some(entry) = entry else { return };
    if !entry.error_popup {
        return;
    }
    let name = if !entry.label.trim().is_empty() {
        entry.label.trim().to_string()
    } else if !entry.command.trim().is_empty() {
        entry.command.trim().to_string()
    } else {
        "process".to_string()
    };
    let all_logs = manager.logs();
    let stderr_lines: Vec<String> = all_logs
        .get(id)
        .map(|ls| {
            ls.iter()
                .filter(|l| l.stream == "stderr")
                .map(|l| l.text.clone())
                .collect()
        })
        .unwrap_or_default();
    let tail: Vec<String> = if stderr_lines.len() > 5 {
        stderr_lines[stderr_lines.len() - 5..].to_vec()
    } else {
        stderr_lines
    };
    let payload = json!({
        "id": id,
        "name": name,
        "exit_code": exit_code,
        "lines": tail.join("\n"),
        "port": port(),
    });
    let arg = payload.to_string();
    match Command::new(exe)
        .arg(arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(_) => {}
        Err(e) => log_to(log_path, &format!("error popup: failed to launch: {e}")),
    }
}

fn start_error_popup_watcher(manager: Arc<Manager>, mut rx: broadcast::Receiver<Event>, log_path: PathBuf) {
    std::thread::spawn(move || loop {
        match rx.blocking_recv() {
            Ok(Event::Status {
                id,
                running,
                exit_code,
                expected,
                ..
            }) => {
                if !running && !expected && exit_code.unwrap_or(0) != 0 {
                    spawn_error_popup(&manager, &id, exit_code, &log_path);
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    });
}

fn start_tray(log_path: &Path) {
    #[cfg(windows)]
    {
        let port = port();
        let ui_exe = find_ui_exe().map(|p| p.display().to_string()).unwrap_or_default();
        let icon = find_icon_path().map(|p| p.display().to_string()).unwrap_or_default();

        fn psq(s: &str) -> String {
            format!("'{}'", s.replace('\'', "''"))
        }

        let mut script = String::new();
        script.push_str("Add-Type -AssemblyName System.Windows.Forms\n");
        script.push_str("Add-Type -AssemblyName System.Drawing\n");
        script.push_str(&format!("$port={port}\n$base='http://127.0.0.1:'+$port\n$uiExe={}\n$iconPath={}\n", psq(&ui_exe), psq(&icon)));
        script.push_str("$notify = New-Object System.Windows.Forms.NotifyIcon\n");
        script.push_str("if ($iconPath -and (Test-Path $iconPath)) { try { $notify.Icon = [System.Drawing.Icon]::ExtractAssociatedIcon($iconPath) } catch { $notify.Icon = [System.Drawing.SystemIcons]::Application } } else { $notify.Icon = [System.Drawing.SystemIcons]::Application }\n");
        script.push_str("$notify.Text = 'On Start Up'\n$notify.Visible = $true\n");
        // Dark theme matching main app
        script.push_str("$bg=[System.Drawing.Color]::FromArgb(21,26,35)\n");
        script.push_str("$bg2=[System.Drawing.Color]::FromArgb(26,33,48)\n");
        script.push_str("$border=[System.Drawing.Color]::FromArgb(38,48,63)\n");
        script.push_str("$text=[System.Drawing.Color]::FromArgb(215,222,233)\n");
        script.push_str("$hover=[System.Drawing.Color]::FromArgb(120,132,150)\n");
        script.push_str("$sel=[System.Drawing.Color]::FromArgb(120,132,150)\n");
        script.push_str("Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing -TypeDefinition @'\n");
        script.push_str("using System.Drawing;\nusing System.Windows.Forms;\n");
        script.push_str("public class DarkColorTable : ProfessionalColorTable {\n");
        script.push_str("  Color bg,bg2,border,hover,sel;\n");
        script.push_str("  public DarkColorTable(Color bg,Color bg2,Color border,Color hover,Color sel){ this.bg=bg; this.bg2=bg2; this.border=border; this.hover=hover; this.sel=sel; }\n");
        script.push_str("  public override Color ToolStripDropDownBackground { get { return bg; } }\n");
        script.push_str("  public override Color ImageMarginGradientBegin { get { return bg; } }\n");
        script.push_str("  public override Color ImageMarginGradientMiddle { get { return bg; } }\n");
        script.push_str("  public override Color ImageMarginGradientEnd { get { return bg; } }\n");
        script.push_str("  public override Color MenuBorder { get { return border; } }\n");
        script.push_str("  public override Color MenuItemBorder { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemSelected { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemSelectedGradientBegin { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemSelectedGradientEnd { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemPressedGradientBegin { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemPressedGradientMiddle { get { return hover; } }\n");
        script.push_str("  public override Color MenuItemPressedGradientEnd { get { return hover; } }\n");
        script.push_str("  public override Color ButtonSelectedHighlight { get { return hover; } }\n");
        script.push_str("  public override Color ButtonSelectedGradientBegin { get { return hover; } }\n");
        script.push_str("  public override Color ButtonSelectedGradientMiddle { get { return hover; } }\n");
        script.push_str("  public override Color ButtonSelectedGradientEnd { get { return hover; } }\n");
        script.push_str("  public override Color ButtonPressedHighlight { get { return hover; } }\n");
        script.push_str("  public override Color ButtonPressedGradientBegin { get { return hover; } }\n");
        script.push_str("  public override Color ButtonPressedGradientMiddle { get { return hover; } }\n");
        script.push_str("  public override Color ButtonPressedGradientEnd { get { return hover; } }\n");
        script.push_str("  public override Color SeparatorDark { get { return border; } }\n");
        script.push_str("  public override Color SeparatorLight { get { return bg2; } }\n");
        script.push_str("}\n'@ -ErrorAction SilentlyContinue\n");
        script.push_str("$ct = New-Object DarkColorTable $bg,$bg2,$border,$hover,$sel\n");
        script.push_str("$renderer = New-Object System.Windows.Forms.ToolStripProfessionalRenderer $ct\n");
        script.push_str("$renderer.RoundedEdges = $false\n");
        script.push_str("[System.Windows.Forms.ToolStripManager]::VisualStylesEnabled = $false\n");
        script.push_str("$menu = New-Object System.Windows.Forms.ContextMenuStrip\n");
        script.push_str("$menu.Renderer = $renderer\n");
        script.push_str("$menu.BackColor = $bg\n$menu.ForeColor = $text\n");
        script.push_str("$menu.ShowImageMargin = $false\n$menu.ShowCheckMargin = $false\n");
        script.push_str("$menu.Padding = New-Object System.Windows.Forms.Padding(2,4,2,4)\n");
        script.push_str("$open = $menu.Items.Add('Open On Start Up')\n");
        script.push_str("$startAll = $menu.Items.Add('Start all processes')\n");
        script.push_str("$stopAll = $menu.Items.Add('Stop all processes')\n");
        script.push_str("[void]$menu.Items.Add('-')\n");
        script.push_str("$exitItem = $menu.Items.Add('Stop service & Exit')\n");
        script.push_str("foreach($it in @($open,$startAll,$stopAll,$exitItem)){ $it.ForeColor=$text; $it.Padding = New-Object System.Windows.Forms.Padding(6,4,6,4) }\n");
        script.push_str("$doOpen = { if ($uiExe -and (Test-Path $uiExe)) { try { $psi = New-Object System.Diagnostics.ProcessStartInfo; $psi.FileName = $uiExe; $psi.WorkingDirectory = (Split-Path -Parent $uiExe); $psi.UseShellExecute = $true; [System.Diagnostics.Process]::Start($psi) | Out-Null } catch {} } }\n");
        script.push_str("$open.add_Click($doOpen)\n$notify.add_DoubleClick($doOpen)\n");
        script.push_str("$notify.add_MouseClick({ param($s,$e) if ($e.Button -eq [System.Windows.Forms.MouseButtons]::Left) { & $doOpen } })\n");
        script.push_str("$startAll.add_Click({ try { Invoke-RestMethod -Uri ($base+'/api/start-all') -Method Post -ContentType 'application/json' -Body '{}' | Out-Null } catch {} })\n");
        script.push_str("$stopAll.add_Click({ try { Invoke-RestMethod -Uri ($base+'/api/stop-all') -Method Post -ContentType 'application/json' -Body '{}' | Out-Null } catch {} })\n");
        script.push_str("$exitItem.add_Click({ try { Invoke-RestMethod -Uri ($base+'/api/shutdown') -Method Post -ContentType 'application/json' -Body '{}' -TimeoutSec 2 | Out-Null } catch {} ; $notify.Visible=$false; [System.Windows.Forms.Application]::Exit() })\n");
        script.push_str("$notify.ContextMenuStrip = $menu\n");
        script.push_str("[System.Windows.Forms.Application]::Run()\n");

        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        match Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-STA",
                "-ExecutionPolicy",
                "Bypass",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            Ok(child) => {
                if let Ok(mut slot) = tray_cell().lock() {
                    *slot = Some(child);
                }
            }
            Err(e) => {
                log_to(log_path, &format!("failed to start tray: {e}"));
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = log_path;
    }
}

fn stop_tray() {
    if let Ok(mut slot) = tray_cell().lock() {
        if let Some(mut child) = slot.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn kill_ui_and_popups() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        for im in ["on-start-up.exe", "on-start-up-error-popup.exe"] {
            let _ = Command::new("taskkill")
                .args(["/IM", im, "/F", "/T"])
                .creation_flags(CREATE_NO_WINDOW)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("pkill").args(["-f", "on-start-up-error-popup"]).status();
        let _ = Command::new("pkill").args(["-x", "on-start-up"]).status();
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let config_dir = config_dir();
    let _ = std::fs::create_dir_all(&config_dir);
    let log_path = config_dir.join("server.log");
    rotate_log(&log_path);

    // Single instance: if the port is taken, another server is already running.
    let addr = SocketAddr::from(([127, 0, 0, 1], port()));
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            log_to(
                &log_path,
                &format!("cannot bind {addr}: {e} â€” assuming another instance is running"),
            );
            return;
        }
    };

    let (event_bus, _) = broadcast::channel::<Event>(1024);
    let sink_events = event_bus.clone();
    let manager = Manager::new(
        Box::new(move |event| {
            let _ = sink_events.send(event);
        }),
        config_dir.join("on-start-up.json"),
    );
    log_to(
        &log_path,
        &format!(
            "listening on {addr}, {} entries loaded",
            manager.entries().len()
        ),
    );

    // This app always auto-starts: keep the login item registered (and pointed
    // at the current location of the launcher, which is the UI shell when
    // available â€” closing the window never stops the service).
    if manager.settings().autostart {
        if let Err(e) = autostart::install(&on_start_up_core::launcher_exe()) {
            log_to(&log_path, &format!("autostart registration failed: {e}"));
        }
    }
    if manager.settings().alias_active {
        let cmd = on_start_up_core::alias_cmd::normalize_command_name(&manager.settings().alias_command);
        if let Err(e) = on_start_up_core::alias_cmd::install(&cmd) {
            log_to(&log_path, &format!("alias activation failed: {e}"));
        }
    }

    // Tray icon belongs to the server (always available while service runs).
    start_tray(&log_path);

    // Error popups are triggered by the server via a separate executable.
    start_error_popup_watcher(Arc::clone(&manager), event_bus.subscribe(), log_path.clone());

    // On start, run every entry triggered by "on start up".
    {
        let manager = Arc::clone(&manager);
        let log_path = log_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(600));
            let errors = manager.start_startup_entries();
            for error in errors {
                log_to(&log_path, &format!("auto-start: {error}"));
            }
            log_to(&log_path, "\"on start up\" entries auto-started");
        });
    }

    // Scheduler: fires time/date triggers (once / daily / weekly).
    {
        let manager = Arc::clone(&manager);
        let log_path = log_path.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(15));
                for entry in manager.entries() {
                    let Some(stamp) = trigger::due_stamp(&entry) else {
                        continue;
                    };
                    if manager.start_as(&entry.id, true).is_ok() {
                        manager.mark_fired(&entry.id, &stamp);
                        log_to(
                            &log_path,
                            &format!(
                                "schedule: started \"{}\" for {stamp}",
                                entry.display_name()
                            ),
                        );
                    }
                }
            }
        });
    }

    let app: App = Arc::new(AppInner {
        manager: Arc::clone(&manager),
        events: event_bus,
    });

    let router = Router::new()
        .route("/api/ping", get(ping))
        .route("/api/state", get(get_state))
        .route("/api/entries", post(add_entry).put(update_entry))
        .route("/api/entries/remove", post(remove_entry))
        .route("/api/aliases", post(add_alias).put(update_alias))
        .route("/api/aliases/remove", post(remove_alias))
        .route("/api/start", post(start))
        .route("/api/stop", post(stop))
        .route("/api/start-all", post(start_all))
        .route("/api/stop-all", post(stop_all))
        .route("/api/logs/clear", post(clear_logs))
        .route("/api/settings", post(set_settings))
        .route("/api/shutdown", post(shutdown))
        .route("/api/events", get(events))
        .layer(CorsLayer::very_permissive())
        .with_state(app);

    log_to(&log_path, "ready");
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await;
    if let Err(e) = result {
        log_to(&log_path, &format!("server error: {e}"));
    }

    log_to(&log_path, "shutting down");
    stop_tray();
    kill_ui_and_popups();
    manager.shutdown();
}
