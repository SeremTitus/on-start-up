import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  Alias,
  AliasPayload,
  Entry,
  EntryPayload,
  EnsureResult,
  ServerInfo,
  ServerState,
  Settings,
  StreamEvent,
} from "./types";

let base = "";

/** Point the client at the background service. */
export function init(url: string) {
  base = `${url}/api`;
}

async function req<T>(path: string, method = "GET", body?: unknown): Promise<T> {
  const res = await fetch(`${base}${path}`, {
    method,
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as Record<string, unknown>;
  if (!res.ok) {
    throw new Error(
      typeof data.error === "string" ? data.error : `request failed (${res.status})`,
    );
  }
  return data as T;
}

// --- service discovery (Tauri side) ---

export const serverInfo = () => invoke<ServerInfo>("server_info");
export const ensureServer = () => invoke<EnsureResult>("ensure_server");

// --- service API (HTTP) ---

export const getState = () => req<ServerState>("/state");

export const addEntry = (payload: EntryPayload) =>
  req<{ entry: Entry }>("/entries", "POST", payload).then((r) => r.entry);

export const updateEntry = (entry: Entry) => req<unknown>("/entries", "PUT", { entry });

export const removeEntry = (id: string) => req<unknown>("/entries/remove", "POST", { id });

export const startEntry = (id: string) => req<unknown>("/start", "POST", { id });

export const stopEntry = (id: string) => req<unknown>("/stop", "POST", { id });

/** Returns error strings for entries that could not be started. */
export const startAll = () =>
  req<{ errors: string[] }>("/start-all", "POST").then((r) => r.errors);

export const stopAll = () => req<unknown>("/stop-all", "POST");

export const clearLogs = (id: string) => req<unknown>("/logs/clear", "POST", { id });

export const addAlias = (payload: AliasPayload) =>
  req<{ alias: Alias }>("/aliases", "POST", payload).then((r) => r.alias);

export const updateAlias = (alias: Alias) => req<unknown>("/aliases", "PUT", alias);

export const removeAlias = (id: string) => req<unknown>("/aliases/remove", "POST", { id });

export const setSettings = (settings: Settings) => req<unknown>("/settings", "POST", settings);

// --- native helpers ---

/** Open a folder picker and return the chosen directory (or null). */
export async function browseDirectory(defaultPath?: string): Promise<string | null> {
  const picked = await open({
    directory: true,
    multiple: false,
    title: "Select working directory",
    defaultPath,
  });
  return typeof picked === "string" ? picked : null;
}

/** Open a directory in the system file manager. */
export const openInFileManager = (path: string) =>
  invoke<void>("open_in_file_manager", { path });

/** Live log/status stream from the service. */
export function openStream(onEvent: (ev: StreamEvent) => void): EventSource {
  const es = new EventSource(`${base}/events`);
  es.onmessage = (e) => {
    try {
      onEvent(JSON.parse(e.data) as StreamEvent);
    } catch {
      /* ignore malformed frames */
    }
  };
  return es;
}
