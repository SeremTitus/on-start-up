use serde::{Deserialize, Serialize};

/// When a process should be started.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Trigger {
    /// Run automatically when the service starts (default).
    Startup,
    /// One-shot run at a date + time. `date` is "YYYY-MM-DD", `time` is "HH:MM" (local time).
    Once { date: String, time: String },
    /// Every day at `time`.
    Daily { time: String },
    /// On the given weekdays (0 = Monday … 6 = Sunday) at `time`.
    Weekly { days: Vec<u8>, time: String },
}

impl Default for Trigger {
    fn default() -> Self {
        Trigger::Startup
    }
}

/// A custom, user-imported sound (stored as a data URL in the config).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSound {
    pub id: String,
    pub name: String,
    pub data: String,
}

/// Optional sound alerts for a process's own lifecycle events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alerts {
    /// Play when the process starts by itself (auto/scheduled start).
    #[serde(default)]
    pub start: bool,
    /// Sound key for starts ("chime" | "ping" | "bell" | "buzz" | "custom:<id>").
    #[serde(default = "default_sound")]
    pub start_sound: String,
    /// Play when the process exits by itself (not stopped by the user).
    #[serde(default)]
    pub exit: bool,
    /// Sound key for exits; may differ from the start sound.
    #[serde(default = "default_sound")]
    pub exit_sound: String,
    /// Sound key played instead of `exit_sound` when the exit is an error.
    /// Empty = same as the exit sound.
    #[serde(default)]
    pub error_sound: String,
    /// User-imported sounds available to either event.
    #[serde(default)]
    pub sounds: Vec<CustomSound>,
}

fn default_sound() -> String {
    "chime".to_string()
}

impl Default for Alerts {
    fn default() -> Self {
        Self {
            start: false,
            start_sound: default_sound(),
            exit: false,
            exit_sound: default_sound(),
            error_sound: String::new(),
            sounds: Vec::new(),
        }
    }
}

/// What to do when a process exits with an error (unexpected, non-zero exit).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ErrorPolicy {
    /// Just show the error state on the entry (default).
    #[default]
    Ignore,
    /// Don't auto-start it again until the user starts it manually.
    StopRuns,
    /// Run it again after a short delay.
    Rerun,
}

/// One managed process: where to run it and what to run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub alerts: Alerts,
    #[serde(default)]
    pub error_policy: ErrorPolicy,
    /// Show a popup notification when the process exits with an error.
    /// Independent of `error_policy`.
    #[serde(default)]
    pub error_popup: bool,
    /// Times to re-run after an error when `error_policy` is `Rerun`;
    /// -1 = infinite.
    #[serde(default = "infinite")]
    pub error_reruns: i32,
    /// Seconds to wait before a re-run (base delay for the backoff).
    #[serde(default = "default_delay")]
    pub rerun_delay: u64,
    /// Grow the re-run delay exponentially (delay × 2^(attempt-1), capped).
    #[serde(default)]
    pub rerun_backoff: bool,
    /// Set when `error_policy` is `StopRuns` and the entry failed: auto-starts
    /// are skipped until the user starts it manually.
    #[serde(default)]
    pub halted: bool,
    /// Stamp of the last scheduled instant that fired; prevents double runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired: Option<String>,
}

fn infinite() -> i32 {
    -1
}

fn default_delay() -> u64 {
    3
}

impl Entry {
    pub fn display_name(&self) -> String {
        let label = self.label.trim();
        if !label.is_empty() {
            label.to_string()
        } else {
            self.command.trim().to_string()
        }
    }
}

/// A single output line captured from a managed process.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogLine {
    /// Monotonic sequence number (used by clients for idempotent merging).
    pub seq: u64,
    /// "stdout" | "stderr" | "sys"
    pub stream: String,
    pub text: String,
    /// Unix timestamp in milliseconds.
    pub ts: u64,
}

/// One command step in a multi-command alias.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AliasCommand {
    #[serde(default)]
    pub command: String,
    /// Whether extra CLI args are appended to this command.
    #[serde(default)]
    pub accepts_args: bool,
}

/// A command alias invokable as `osu <name> [args...]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alias {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub commands: Vec<AliasCommand>,
    #[serde(default)]
    pub dir: String,
}

impl Alias {
    /// Trim fields and ensure at least one command receives extra args.
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_string();
        self.dir = self.dir.trim().to_string();
        for c in &mut self.commands {
            c.command = c.command.trim().to_string();
        }
        self.commands.retain(|c| !c.command.is_empty());
        if !self.commands.is_empty() && !self.commands.iter().any(|c| c.accepts_args) {
            if let Some(last) = self.commands.last_mut() {
                last.accepts_args = true;
            }
        }
    }
}

/// Everything the server pushes to connected UIs.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Event {
    Log { id: String, line: LogLine },
    Status {
        id: String,
        running: bool,
        exit_code: Option<i32>,
        /// The user asked for this state change (e.g. a Stop click).
        expected: bool,
        /// Started on its own (boot/schedule) rather than by the user.
        auto: bool,
    },
}

fn default_alias_command() -> String {
    "osu".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Register the server to start automatically when the user logs in.
    #[serde(default = "default_true")]
    pub autostart: bool,
    /// Register the command alias shim on the user's PATH.
    #[serde(default)]
    pub alias_active: bool,
    /// The CLI command name registered on PATH (default: "osu").
    #[serde(default = "default_alias_command")]
    pub alias_command: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: true,
            alias_active: false,
            alias_command: default_alias_command(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// On-disk representation of the config file.
#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Persisted {
    #[serde(default = "default_true")]
    pub autostart: bool,
    #[serde(default)]
    pub alias_active: bool,
    #[serde(default = "default_alias_command")]
    pub alias_command: String,
    #[serde(default)]
    pub entries: Vec<Entry>,
    #[serde(default)]
    pub aliases: Vec<Alias>,
}
