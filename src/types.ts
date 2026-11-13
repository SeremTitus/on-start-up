/** When a process should run. */
export type Trigger =
  | { kind: "startup" }
  | { kind: "once"; date: string; time: string }
  | { kind: "daily"; time: string }
  | { kind: "weekly"; days: number[]; time: string };

/** A custom, user-imported sound (data URL). */
export interface CustomSound {
  id: string;
  name: string;
  data: string;
}

/** Optional sound alerts for a process's own lifecycle events. */
export interface Alerts {
  start: boolean;
  startSound: string;
  exit: boolean;
  exitSound: string;
  /** Played instead of `exitSound` when the exit is an error; "" = same. */
  errorSound: string;
  sounds: CustomSound[];
}

/** What to do when a process exits with an error. */
export type ErrorPolicy = "ignore" | "stopRuns" | "rerun";

export interface Entry {
  id: string;
  label: string;
  dir: string;
  command: string;
  trigger: Trigger;
  alerts: Alerts;
  errorPolicy: ErrorPolicy;
  /** Show a popup on error; independent of `errorPolicy`. */
  errorPopup: boolean;
  /** Times to re-run after an error for the "rerun" policy; -1 = infinite. */
  errorReruns: number;
  /** Seconds to wait before a re-run (base delay for the backoff). */
  rerunDelay: number;
  /** Grow the re-run delay exponentially. */
  rerunBackoff: boolean;
  /** Set by "stop future runs" after an error; manual start resumes. */
  halted: boolean;
  lastFired?: string;
}

/** Payload for creating an entry (the service assigns id and bookkeeping). */
export type EntryPayload = Omit<Entry, "id" | "lastFired" | "halted">;

export type LogStream = "stdout" | "stderr" | "sys";

export interface LogLine {
  /** Monotonic id, used to merge snapshots and live events without duplicates. */
  seq: number;
  stream: LogStream;
  text: string;
  /** Unix timestamp in milliseconds. */
  ts: number;
}

export interface AliasCommand {
  command: string;
  acceptsArgs: boolean;
}

export interface Alias {
  id: string;
  name: string;
  commands: AliasCommand[];
  dir: string;
}

export type AliasPayload = Omit<Alias, "id">;

export interface Settings {
  /** Register the background service to start when the user logs in. */
  autostart: boolean;
  /** Register the command alias shim on the user's PATH. */
  aliasActive: boolean;
  /** CLI command name registered on PATH (default: "osu"). */
  aliasCommand: string;
}

export interface ServerState {
  entries: Entry[];
  aliases: Alias[];
  logs: Record<string, LogLine[]>;
  running: Record<string, boolean>;
  /** Entries whose last run ended with an error (unexpected, non-zero exit). */
  exits: Record<string, number | null>;
  settings: Settings;
}

export type StreamEvent =
  | { type: "hello" }
  | { type: "lagged"; count: number }
  | { type: "log"; id: string; line: LogLine }
  | {
      type: "status";
      id: string;
      running: boolean;
      exitCode: number | null;
      /** The user asked for this change (e.g. a Stop click). */
      expected: boolean;
      /** Started on its own (boot/schedule) rather than by the user. */
      auto: boolean;
    };

export interface ServerInfo {
  url: string;
  port: number;
  serverExe: string | null;
}

export interface EnsureResult {
  running: boolean;
  started: boolean;
  error?: string;
}
