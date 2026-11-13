use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::model::{Alias, Entry, ErrorPolicy, Event, LogLine, Persisted, Settings, Trigger};
use crate::Sink;

const MAX_LOG_LINES: usize = 5000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct ProcHandle {
    child: Arc<Mutex<Child>>,
    pid: u32,
    /// Set when the user asks to stop: distinguishes "exited by itself".
    stop_requested: Arc<AtomicBool>,
}

/// Owns the entry list, the spawned processes and their captured output.
///
/// Lives behind an [`Arc`] because every spawned reader/monitor thread needs a
/// handle back to it. All process state is behind short-lived mutexes, so it is
/// safe to call from multiple threads (or async tasks).
pub struct Manager {
    entries: Mutex<Vec<Entry>>,
    aliases: Mutex<Vec<Alias>>,
    logs: Mutex<HashMap<String, Vec<LogLine>>>,
    procs: Mutex<HashMap<String, ProcHandle>>,
    /// Entries whose last run ended with an error (unexpected, non-zero exit).
    last_exits: Mutex<HashMap<String, Option<i32>>>,
    /// Error re-runs performed since the last manual start (per entry).
    rerun_progress: Mutex<HashMap<String, i32>>,
    settings: Mutex<Settings>,
    config_path: PathBuf,
    seq: AtomicU64,
    sink: Sink,
}

impl Manager {
    /// Create a manager and load persisted state from `config_path`.
    pub fn new(sink: Sink, config_path: PathBuf) -> Arc<Self> {
        let manager = Arc::new(Self {
            entries: Mutex::new(Vec::new()),
            aliases: Mutex::new(Vec::new()),
            logs: Mutex::new(HashMap::new()),
            procs: Mutex::new(HashMap::new()),
            last_exits: Mutex::new(HashMap::new()),
            rerun_progress: Mutex::new(HashMap::new()),
            settings: Mutex::new(Settings::default()),
            config_path,
            // Seed from the clock so sequence numbers stay unique across
            // service restarts (clients merge snapshots with live events).
            seq: AtomicU64::new(now_ms().saturating_mul(1000)),
            sink,
        });
        manager.load();
        manager
    }

    // -----------------------------------------------------------------
    // Events & logs
    // -----------------------------------------------------------------

    fn emit(&self, event: Event) {
        (self.sink)(event);
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn push_log(&self, id: &str, stream: &str, text: String) {
        let line = LogLine {
            seq: self.next_seq(),
            stream: stream.to_string(),
            text,
            ts: now_ms(),
        };
        if let Ok(mut logs) = self.logs.lock() {
            let buf = logs.entry(id.to_string()).or_default();
            if buf.len() >= MAX_LOG_LINES {
                let excess = buf.len() + 1 - MAX_LOG_LINES;
                buf.drain(0..excess);
            }
            buf.push(line.clone());
        }
        self.emit(Event::Log {
            id: id.to_string(),
            line,
        });
    }

    fn sys(&self, id: &str, text: String) {
        self.push_log(id, "sys", text);
    }

    pub fn logs(&self) -> HashMap<String, Vec<LogLine>> {
        self.logs.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn clear_logs(&self, id: &str) {
        if let Ok(mut logs) = self.logs.lock() {
            logs.remove(id);
        }
        self.sys(id, "log cleared".to_string());
    }

    // -----------------------------------------------------------------
    // Persistence
    // -----------------------------------------------------------------

    fn load(&self) {
        let Ok(text) = std::fs::read_to_string(&self.config_path) else {
            return;
        };
        let Ok(persisted) = serde_json::from_str::<Persisted>(&text) else {
            return;
        };
        if let Ok(mut entries) = self.entries.lock() {
            *entries = persisted.entries;
        }
        if let Ok(mut aliases) = self.aliases.lock() {
            *aliases = persisted.aliases;
        }
        if let Ok(mut settings) = self.settings.lock() {
            *settings = Settings {
                autostart: persisted.autostart,
                alias_active: persisted.alias_active,
                alias_command: if persisted.alias_command.trim().is_empty() {
                    "osu".to_string()
                } else {
                    persisted.alias_command
                },
            };
        }
    }

    pub fn save(&self) {
        let (autostart, alias_active, alias_command) = self
            .settings
            .lock()
            .map(|s| (s.autostart, s.alias_active, s.alias_command.clone()))
            .unwrap_or((true, false, "osu".to_string()));
        let persisted = Persisted {
            autostart,
            alias_active,
            alias_command,
            entries: self.entries.lock().map(|e| e.clone()).unwrap_or_default(),
            aliases: self.aliases.lock().map(|a| a.clone()).unwrap_or_default(),
        };
        if let Some(parent) = self.config_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&persisted) {
            let _ = std::fs::write(&self.config_path, json);
        }
    }

    // -----------------------------------------------------------------
    // Entries
    // -----------------------------------------------------------------

    pub fn entries(&self) -> Vec<Entry> {
        self.entries.lock().map(|e| e.clone()).unwrap_or_default()
    }

    pub fn add_entry(&self, mut entry: Entry) -> Entry {
        entry.id = format!("e{}-{}", now_ms(), self.next_seq());
        entry.halted = false;
        entry.last_fired = None;
        if let Ok(mut entries) = self.entries.lock() {
            entries.push(entry.clone());
        }
        self.save();
        self.sys(&entry.id, "entry added".to_string());
        entry
    }

    pub fn update_entry(&self, entry: Entry) -> Result<(), String> {
        {
            let mut entries = self.entries.lock().map_err(|_| "state poisoned".to_string())?;
            let existing = entries
                .iter_mut()
                .find(|e| e.id == entry.id)
                .ok_or_else(|| "entry not found".to_string())?;
            *existing = entry;
        }
        self.save();
        Ok(())
    }

    pub fn remove_entry(&self, id: &str) -> Result<(), String> {
        if self.is_running(id) {
            return Err("stop the process before removing it".into());
        }
        {
            let mut entries = self.entries.lock().map_err(|_| "state poisoned".to_string())?;
            let before = entries.len();
            entries.retain(|e| e.id != id);
            if entries.len() == before {
                return Err("entry not found".into());
            }
        }
        if let Ok(mut logs) = self.logs.lock() {
            logs.remove(id);
        }
        self.save();
        Ok(())
    }

    // -----------------------------------------------------------------
    // Aliases
    // -----------------------------------------------------------------

    pub fn aliases(&self) -> Vec<Alias> {
        self.aliases.lock().map(|a| a.clone()).unwrap_or_default()
    }

    pub fn add_alias(&self, mut alias: Alias) -> Result<Alias, String> {
        alias.normalize();
        if alias.name.is_empty() {
            return Err("alias name is required".into());
        }
        if alias.commands.is_empty() {
            return Err("at least one command is required".into());
        }
        if alias
            .name
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\\')
        {
            return Err("alias name cannot contain spaces or slashes".into());
        }
        let mut aliases = self.aliases.lock().map_err(|_| "state poisoned".to_string())?;
        if aliases.iter().any(|a| a.name.eq_ignore_ascii_case(&alias.name)) {
            return Err("alias name already exists".into());
        }
        alias.id = format!("a{}-{}", now_ms(), self.next_seq());
        aliases.push(alias.clone());
        drop(aliases);
        self.save();
        Ok(alias)
    }

    pub fn update_alias(&self, mut alias: Alias) -> Result<(), String> {
        alias.normalize();
        if alias.name.is_empty() {
            return Err("alias name is required".into());
        }
        if alias.commands.is_empty() {
            return Err("at least one command is required".into());
        }
        if alias
            .name
            .chars()
            .any(|c| c.is_whitespace() || c == '/' || c == '\\')
        {
            return Err("alias name cannot contain spaces or slashes".into());
        }
        let mut aliases = self.aliases.lock().map_err(|_| "state poisoned".to_string())?;
        if aliases
            .iter()
            .any(|a| a.id != alias.id && a.name.eq_ignore_ascii_case(&alias.name))
        {
            return Err("alias name already exists".into());
        }
        let existing = aliases
            .iter_mut()
            .find(|a| a.id == alias.id)
            .ok_or_else(|| "alias not found".to_string())?;
        *existing = alias;
        drop(aliases);
        self.save();
        Ok(())
    }

    pub fn remove_alias(&self, id: &str) -> Result<(), String> {
        let mut aliases = self.aliases.lock().map_err(|_| "state poisoned".to_string())?;
        let before = aliases.len();
        aliases.retain(|a| a.id != id);
        if aliases.len() == before {
            return Err("alias not found".into());
        }
        drop(aliases);
        self.save();
        Ok(())
    }

    // -----------------------------------------------------------------
    // Settings
    // -----------------------------------------------------------------

    pub fn settings(&self) -> Settings {
        self.settings.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut s) = self.settings.lock() {
            *s = settings;
        }
        self.save();
    }

    // -----------------------------------------------------------------
    // Process lifecycle
    // -----------------------------------------------------------------

    pub fn is_running(&self, id: &str) -> bool {
        self.procs.lock().map(|p| p.contains_key(id)).unwrap_or(false)
    }

    pub fn running(&self) -> HashMap<String, bool> {
        self.procs
            .lock()
            .map(|p| p.keys().map(|k| (k.clone(), true)).collect())
            .unwrap_or_default()
    }

    /// Entries whose last run ended with an error (unexpected, non-zero exit).
    pub fn exits(&self) -> HashMap<String, Option<i32>> {
        self.last_exits
            .lock()
            .map(|e| e.clone())
            .unwrap_or_default()
    }

    /// Start one entry (user-initiated). Captured output is pushed through the
    /// sink line by line as it arrives.
    pub fn start(self: &Arc<Self>, id: &str) -> Result<(), String> {
        self.start_as(id, false)
    }

    /// Start one entry; `auto` marks starts that happen on their own (boot or
    /// schedule) so clients can treat them differently from user clicks.
    pub fn start_as(self: &Arc<Self>, id: &str, auto: bool) -> Result<(), String> {
        if self.is_running(id) {
            return Err("already running".into());
        }

        let entry = self
            .entries
            .lock()
            .map_err(|_| "state poisoned".to_string())?
            .iter()
            .find(|e| e.id == id)
            .cloned()
            .ok_or_else(|| "entry not found".to_string())?;

        if auto && entry.halted {
            return Err("runs halted after an error — start it manually to resume".into());
        }

        let command = entry.command.trim().to_string();
        if command.is_empty() {
            return Err("command is empty".into());
        }
        if !entry.dir.trim().is_empty() && !Path::new(entry.dir.trim()).is_dir() {
            return Err(format!("directory does not exist: {}", entry.dir));
        }
        // Commands run through the platform shell so things like
        // "npm run dev" or "python main.py" work verbatim.
        let mut cmd = {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let mut c = Command::new("cmd");
                c.args(["/C", &command]);
                c.creation_flags(CREATE_NO_WINDOW);
                c
            }
            #[cfg(not(windows))]
            {
                use std::os::unix::process::CommandExt;
                let mut c = Command::new("sh");
                c.args(["-c", &command]);
                // Own process group so the whole tree can be killed on stop.
                c.process_group(0);
                c
            }
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Working directory is optional: empty means the user's home directory.
        let workdir = if entry.dir.trim().is_empty() {
            dirs::home_dir()
        } else {
            Some(PathBuf::from(entry.dir.trim()))
        };
        if let Some(workdir) = workdir {
            cmd.current_dir(workdir);
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                let msg = format!("failed to spawn: {e}");
                self.sys(id, msg.clone());
                return Err(msg);
            }
        };

        let pid = child.id();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(child));
        let stop_requested = Arc::new(AtomicBool::new(false));

        if let Ok(mut procs) = self.procs.lock() {
            procs.insert(
                id.to_string(),
                ProcHandle {
                    child: child.clone(),
                    pid,
                    stop_requested: stop_requested.clone(),
                },
            );
        }
        if let Ok(mut exits) = self.last_exits.lock() {
            exits.remove(id);
        }
        if !auto {
            // A manual start resets the error re-run budget and resumes an
            // entry halted by "stop future runs".
            if let Ok(mut p) = self.rerun_progress.lock() {
                p.remove(id);
            }
            if entry.halted {
                if let Ok(mut entries) = self.entries.lock() {
                    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                        e.halted = false;
                    }
                }
                self.save();
            }
        }

        if let Some(out) = stdout {
            self.spawn_reader(id.to_string(), "stdout", out);
        }
        if let Some(err) = stderr {
            self.spawn_reader(id.to_string(), "stderr", err);
        }

        self.sys(id, format!("started (pid {pid})"));
        self.emit(Event::Status {
            id: id.to_string(),
            running: true,
            exit_code: None,
            expected: false,
            auto,
        });

        // Watch for exit in the background so the UI always gets a final event.
        let manager = Arc::clone(self);
        let id = id.to_string();
        std::thread::spawn(move || {
            let exit_code = loop {
                let mut result: Option<Option<i32>> = None;
                if let Ok(mut c) = child.lock() {
                    match c.try_wait() {
                        Ok(Some(status)) => result = Some(status.code()),
                        Ok(None) => {}
                        Err(_) => result = Some(None),
                    }
                }
                if let Some(code) = result {
                    break code;
                }
                std::thread::sleep(Duration::from_millis(200));
            };

            if let Ok(mut procs) = manager.procs.lock() {
                let stale = procs.get(&id).map(|p| p.pid == pid).unwrap_or(false);
                if stale {
                    procs.remove(&id);
                }
            }

            let expected = stop_requested.load(Ordering::Relaxed);
            {
                // Remember unexpected error exits (red dot in the UI).
                let mut exits = manager.last_exits.lock().unwrap_or_else(|e| e.into_inner());
                if !expected && exit_code != Some(0) {
                    exits.insert(id.clone(), exit_code);
                } else {
                    exits.remove(&id);
                }
            }

            // Apply the entry's on-error policy.
            if !expected && exit_code != Some(0) {
                let entry = manager.entries().into_iter().find(|e| e.id == id);
                if let Some(entry) = entry {
                    match entry.error_policy {
                        ErrorPolicy::StopRuns => {
                            if let Ok(mut entries) = manager.entries.lock() {
                                if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                                    e.halted = true;
                                }
                            }
                            manager.save();
                            manager.sys(
                                &id,
                                "runs halted after error — start manually to resume".to_string(),
                            );
                        }
                        ErrorPolicy::Rerun => {
                            let done = manager
                                .rerun_progress
                                .lock()
                                .map(|p| p.get(&id).copied().unwrap_or(0))
                                .unwrap_or(0);
                            if entry.error_reruns < 1 || done < entry.error_reruns {
                                if let Ok(mut p) = manager.rerun_progress.lock() {
                                    p.insert(id.clone(), done + 1);
                                }
                                // Wait before re-running (optionally growing
                                // exponentially) so a crash loop doesn't hammer
                                // the machine.
                                let attempt = done + 1;
                                let mut delay = entry.rerun_delay;
                                if entry.rerun_backoff {
                                    delay = delay
                                        .saturating_mul(1u64 << (attempt.min(6) - 1).min(5));
                                }
                                let delay = delay.min(600);
                                manager.sys(
                                    &id,
                                    format!("re-running after error ({attempt}) in {delay}s"),
                                );
                                let m = Arc::clone(&manager);
                                let rid = id.clone();
                                std::thread::spawn(move || {
                                    std::thread::sleep(Duration::from_secs(delay));
                                    let _ = m.start_as(&rid, true);
                                });
                            } else {
                                // Circuit breaker: the fixed rerun budget is
                                // spent — give up and stop future runs.
                                if let Ok(mut entries) = manager.entries.lock() {
                                    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                                        e.halted = true;
                                    }
                                }
                                manager.save();
                                manager.sys(
                                    &id,
                                    "rerun limit reached — future runs stopped (start manually to resume)"
                                        .to_string(),
                                );
                            }
                        }
                        ErrorPolicy::Ignore => {}
                    }
                }
            }

            let text = match exit_code {
                Some(code) => format!("process exited with code {code}"),
                None => "process exited".to_string(),
            };
            manager.sys(&id, text);
            manager.emit(Event::Status {
                id,
                running: false,
                exit_code,
                expected,
                auto: false,
            });
        });

        Ok(())
    }

    pub fn stop(&self, id: &str) -> Result<(), String> {
        let handle = self
            .procs
            .lock()
            .map_err(|_| "state poisoned".to_string())?
            .get(id)
            .map(|p| (p.child.clone(), p.pid, p.stop_requested.clone()))
            .ok_or_else(|| "not running".to_string())?;
        let (child, pid, stop_requested) = handle;
        stop_requested.store(true, Ordering::Relaxed);
        kill_tree(pid, &child);
        self.sys(id, "stop requested".to_string());
        Ok(())
    }

    /// Start every entry (manual "Start all"); returns errors for failures.
    pub fn start_all(self: &Arc<Self>) -> Vec<String> {
        self.start_matching(|e| !e.command.trim().is_empty(), false)
    }

    /// Start the entries triggered by service start — used once at boot.
    pub fn start_startup_entries(self: &Arc<Self>) -> Vec<String> {
        self.start_matching(
            |e| {
                e.trigger == Trigger::Startup
                    && !e.command.trim().is_empty()
                    && !e.halted
            },
            true,
        )
    }

    fn start_matching(
        self: &Arc<Self>,
        filter: impl Fn(&Entry) -> bool,
        auto: bool,
    ) -> Vec<String> {
        let ids: Vec<String> = self
            .entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| filter(e))
                    .map(|e| e.id.clone())
                    .collect()
            })
            .unwrap_or_default();

        let mut errors = Vec::new();
        for id in ids {
            if let Err(e) = self.start_as(&id, auto) {
                errors.push(format!("{id}: {e}"));
            }
        }
        errors
    }

    /// Record that a scheduled instant fired, so it never fires twice.
    pub fn mark_fired(&self, id: &str, stamp: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                entry.last_fired = Some(stamp.to_string());
            }
        }
        self.save();
    }

    pub fn stop_all(&self) {
        let ids: Vec<String> = self
            .procs
            .lock()
            .map(|p| p.keys().cloned().collect())
            .unwrap_or_default();
        for id in ids {
            let _ = self.stop(&id);
        }
    }

    /// Stop everything and wait briefly so shells don't outlive us.
    pub fn shutdown(&self) {
        self.stop_all();
        for _ in 0..20 {
            if self.procs.lock().map(|p| p.is_empty()).unwrap_or(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // -----------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------

    fn spawn_reader(self: &Arc<Self>, id: String, stream: &'static str, reader: impl Read + Send + 'static) {
        let manager = Arc::clone(self);
        std::thread::spawn(move || {
            for line in BufReader::new(reader).lines() {
                match line {
                    Ok(text) => manager.push_log(&id, stream, text),
                    Err(_) => break,
                }
            }
        });
    }
}

/// Kill a spawned command and everything below it.
///
/// Commands run through a shell, so killing just the shell would leak the real
/// program. On Windows `taskkill /T` takes the whole tree down; elsewhere we
/// fall back to killing the shell child directly.
#[cfg(windows)]
fn kill_tree(pid: u32, child: &Arc<Mutex<Child>>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let ok = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        if let Ok(mut c) = child.lock() {
            let _ = c.kill();
        }
    }
}

#[cfg(not(windows))]
fn kill_tree(pid: u32, child: &Arc<Mutex<Child>>) {
    // The shell runs as its own process group (pgid = pid), so signalling the
    // group takes the whole tree down instead of orphaning grandchildren.
    let _ = Command::new("kill")
        .args(["-TERM", &format!("-{pid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if let Ok(mut c) = child.lock() {
        let _ = c.kill();
    }
}
