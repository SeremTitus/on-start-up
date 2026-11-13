import { useEffect, useMemo, useRef, useState } from "react";
import * as api from "./api";
import type {
  Alias,
  AliasCommand,
  Alerts,
  CustomSound,
  Entry,
  ErrorPolicy,
  LogLine,
  ServerState,
  Settings,
  StreamEvent,
  Trigger,
} from "./types";
import {
  onPlaybackChange,
  playSound,
  stopSound,
  TONE_LIST,
  unlockAudio,
} from "./sound";
import { ArrowUpRightIcon, BackIcon, ChevronDownIcon, ChevronUpIcon, DuplicateIcon, InfinityIcon, PencilIcon, ShieldIcon } from "./icons";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { sendNotification } from "@tauri-apps/plugin-notification";

const MAX_LINES = 5000;
const FLUSH_MS = 120;
const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/** Human-readable trigger, e.g. "every day 09:00". */
function triggerLabel(t: Trigger): string {
  switch (t.kind) {
    case "startup":
      return "on start up";
    case "daily":
      return `every day ${t.time}`;
    case "weekly":
      return `${t.days.map((d) => DAY_NAMES[d] ?? "?").join(", ")} ${t.time}`;
    case "once":
      return `${t.date} ${t.time}`;
  }
}

function policyLabel(p: ErrorPolicy): string {
  if (p === "stopRuns") return "On error: stop future runs";
  if (p === "rerun") return "On error: rerun";
  return "On error: default";
}

type PopupData = {
  id: string;
  name: string;
  exitCode: number | null;
  lines: string[];
};

function ErrorPopupView() {
  const params = getViewParams();
  const initial: PopupData = {
    id: params.get("id") ?? "",
    name: params.get("name") ?? "Process",
    exitCode:
      params.get("code") != null && params.get("code") !== ""
        ? Number(params.get("code"))
        : null,
    lines: (params.get("lines") ?? "")
      .split("\n")
      .filter((l) => l.length > 0),
  };
  const [data, setData] = useState<PopupData>(initial);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        // Prefer payload from backend (reliable for bundled windows).
        if (isErrorPopupWindow() && !initial.id) {
          const payload = await invoke<{
            id: string;
            name: string;
            exitCode: number | null;
            lines: string;
          } | null>("get_error_popup");
          if (payload) {
            setData({
              id: payload.id,
              name: payload.name || "Process",
              exitCode: payload.exitCode ?? null,
              lines: (payload.lines || "").split("\n").filter((l) => l.length > 0),
            });
          }
        }
        const info = await api.serverInfo();
        api.init(info.url);
        await api.ensureServer();
      } catch (e) {
        setErr(String(e));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const close = async () => {
    try {
      await getCurrentWindow().close();
    } catch {
      window.close();
    }
  };

  const rerun = async () => {
    if (!data.id) return;
    setBusy(true);
    try {
      await api.startEntry(data.id);
      await close();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  const stopFutureRuns = async () => {
    if (!data.id) return;
    setBusy(true);
    try {
      const state = await api.getState();
      const entry = state.entries.find((e) => e.id === data.id);
      if (entry) {
        await api.updateEntry({ ...entry, errorPolicy: "stopRuns", halted: true });
      }
      await close();
    } catch (e) {
      setErr(String(e));
      setBusy(false);
    }
  };

  return (
    <div className="popup-root" style={{ background: "var(--bg, #0e1117)", color: "var(--text, #d7dee9)" }}>
      <div className="error-modal popup-card">
        <h2>Process failed</h2>
        <div className="error-modal-name">
          <ShieldIcon className="icon" />
          <strong>{data.name}</strong>
          <span className="error-modal-code">exit code {data.exitCode ?? "?"}</span>
        </div>
        {data.lines.length > 0 && (
          <pre className="error-modal-out">{data.lines.join("\n")}</pre>
        )}
        {err && <div className="error-bar">⚠ {err}</div>}
        <div className="modal-actions">
          <span />
          <div className="modal-actions-right">
            <button type="button" className="btn" onClick={close} disabled={busy}>
              Close
            </button>
            <button type="button" className="btn" onClick={stopFutureRuns} disabled={busy || !data.id}>
              Stop future runs
            </button>
            <button type="button" className="btn primary" onClick={rerun} disabled={busy || !data.id}>
              Rerun
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

const pad2 = (n: number) => String(n).padStart(2, "0");
const wrap = (n: number, max: number) => ((n % max) + max) % max;

function TimePicker({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [mode12, setMode12] = useState(false); // display toggle; stored value always 24h
  const [phase, setPhase] = useState<"hour" | "minute">("hour");
  const ref = useRef<HTMLDivElement>(null);
  const dialRef = useRef<SVGSVGElement>(null);
  const dragging = useRef(false);

  const [h, m] = (value || "00:00").split(":").map((x) => parseInt(x, 10));
  const hour = Number.isFinite(h) ? wrap(h, 24) : 0;
  const minute = Number.isFinite(m) ? wrap(m, 60) : 0;

  useEffect(() => {
    if (!open) return;
    setPhase("hour");
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  const commit = (nh: number, nm: number) => onChange(`${pad2(wrap(nh, 24))}:${pad2(wrap(nm, 60))}`);
  const hour12 = hour % 12 === 0 ? 12 : hour % 12;
  const am = hour < 12;
  const setHour12 = (h12: number, isAm: boolean) => {
    const base = h12 % 12; // 12 -> 0
    commit(isAm ? base : base + 12, minute);
  };
  const display = mode12
    ? `${pad2(hour12)}:${pad2(minute)} ${am ? "AM" : "PM"}`
    : `${pad2(hour)}:${pad2(minute)}`;

  const size = 220;
  const cx = size / 2;
  const cy = size / 2;
  const outerR = 88;
  const innerR = 58;

  const pos = (val: number, total: number, r: number) => {
    const ang = (val / total) * Math.PI * 2 - Math.PI / 2;
    return { x: cx + Math.cos(ang) * r, y: cy + Math.sin(ang) * r, ang };
  };

  const selectFromPointer = (clientX: number, clientY: number, commitPhase?: boolean) => {
    const svg = dialRef.current;
    if (!svg) return;
    const rect = svg.getBoundingClientRect();
    const x = clientX - rect.left - cx;
    const y = clientY - rect.top - cy;
    let ang = Math.atan2(y, x) + Math.PI / 2;
    if (ang < 0) ang += Math.PI * 2;
    const dist = Math.sqrt(x * x + y * y);

    if (phase === "hour") {
      if (mode12) {
        const v = Math.round((ang / (Math.PI * 2)) * 12) % 12;
        const h12v = v === 0 ? 12 : v;
        const base = h12v % 12;
        commit(am ? base : base + 12, minute);
      } else {
        const useInner = dist < (outerR + innerR) / 2;
        const v = Math.round((ang / (Math.PI * 2)) * 12) % 12;
        let nh: number;
        if (useInner) {
          // inner ring: 0,13,14,...,23 (v=0 -> 0)
          nh = v === 0 ? 0 : v + 12;
        } else {
          // outer ring: 12,1,2,...,11 (v=0 -> 12)
          nh = v === 0 ? 12 : v;
        }
        commit(nh, minute);
      }
      if (commitPhase) setPhase("minute");
    } else {
      const v = Math.round((ang / (Math.PI * 2)) * 60) % 60;
      commit(hour, v);
      if (commitPhase) setOpen(false);
    }
  };

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    dragging.current = true;
    (e.target as Element).setPointerCapture?.(e.pointerId);
    selectFromPointer(e.clientX, e.clientY, false);
  };
  const onPointerMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!dragging.current) return;
    selectFromPointer(e.clientX, e.clientY, false);
  };
  const onPointerUp = (e: React.PointerEvent<SVGSVGElement>) => {
    if (!dragging.current) return;
    dragging.current = false;
    selectFromPointer(e.clientX, e.clientY, true);
  };

  const onWheel = (e: React.WheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const dir = e.deltaY < 0 ? 1 : -1;
    if (phase === "hour") {
      commit(hour + dir, minute);
    } else {
      commit(hour, minute + dir);
    }
  };

  const hourItems: { label: string; value: number; r: number }[] = [];
  if (phase === "hour") {
    if (mode12) {
      for (let i = 1; i <= 12; i++) hourItems.push({ label: String(i), value: i, r: outerR });
    } else {
      for (let i = 1; i <= 12; i++) hourItems.push({ label: String(i === 12 ? 12 : i), value: i === 12 ? 12 : i, r: outerR });
      for (let i = 0; i < 12; i++) {
        const val = i === 0 ? 0 : i + 12;
        hourItems.push({ label: pad2(val), value: val, r: innerR });
      }
    }
  }

  const minuteItems: number[] = phase === "minute" ? Array.from({ length: 12 }, (_, i) => i * 5) : [];

  const activeHour = mode12 ? hour12 : hour;
  const activePos =
    phase === "hour"
      ? mode12
        ? pos(hour12 % 12, 12, outerR)
        : (hour === 0 || hour > 12 ? pos((hour === 0 ? 0 : hour - 12) % 12, 12, innerR) : pos((hour % 12), 12, outerR))
      : pos(minute, 60, outerR);

  return (
    <div className="picker" ref={ref}>
      <button type="button" className="picker-field" onClick={() => setOpen((o) => !o)}>
        <span className="picker-value">{display}</span>
        <span className="picker-icon" aria-hidden>🕘</span>
      </button>
      {open && (
        <div className="picker-pop time-pop">
          <div className="time-top">
            <div className="time-tabs">
              <button
                type="button"
                className={`time-tab ${phase === "hour" ? "on" : ""}`}
                onClick={() => setPhase("hour")}
              >
                {mode12 ? `${pad2(hour12)} ${am ? "AM" : "PM"}` : pad2(hour)}
              </button>
              <span className="time-sep">:</span>
              <button
                type="button"
                className={`time-tab ${phase === "minute" ? "on" : ""}`}
                onClick={() => setPhase("minute")}
              >
                {pad2(minute)}
              </button>
            </div>
            <button
              type="button"
              className="btn small"
              onClick={() => setMode12((v) => !v)}
              title="Toggle 12/24 hour view (saved as 24-hour)"
            >
              {mode12 ? "24h" : "12h"}
            </button>
          </div>

          {mode12 && phase === "hour" && (
            <div className="ampm-row">
              <button type="button" className={`ampm ${am ? "on" : ""}`} onClick={() => setHour12(hour12, true)}>AM</button>
              <button type="button" className={`ampm ${!am ? "on" : ""}`} onClick={() => setHour12(hour12, false)}>PM</button>
            </div>
          )}

          <div className="dial-wrap" onWheel={onWheel}>
            <svg
              ref={dialRef}
              className="dial"
              width={size}
              height={size}
              viewBox={`0 0 ${size} ${size}`}
              onPointerDown={onPointerDown}
              onPointerMove={onPointerMove}
              onPointerUp={onPointerUp}
              onPointerLeave={() => (dragging.current = false)}
            >
              <circle cx={cx} cy={cy} r={outerR + 16} className="dial-face" />
              <line x1={cx} y1={cy} x2={activePos.x} y2={activePos.y} className="dial-hand" />
              <circle cx={activePos.x} cy={activePos.y} r="16" className="dial-sel" />
              <circle cx={cx} cy={cy} r="3" className="dial-center" />

              {phase === "hour" &&
                hourItems.map((it, idx) => {
                  const p = pos((it.value % 12), 12, it.r);
                  const selected = mode12 ? (it.value % 12) === (activeHour % 12) && it.r === outerR : it.value === hour;
                  return (
                    <text
                      key={idx}
                      x={p.x}
                      y={p.y}
                      className={`dial-num ${selected ? "on" : ""} ${!mode12 && it.r === innerR ? "inner" : ""}`}
                    >
                      {it.label}
                    </text>
                  );
                })}

              {phase === "minute" &&
                minuteItems.map((val) => {
                  const p = pos(val, 60, outerR);
                  const exact = val === minute;
                  return (
                    <g key={val}>
                      <text x={p.x} y={p.y} className={`dial-num ${exact ? "on" : ""}`}>
                        {pad2(val)}
                      </text>
                    </g>
                  );
                })}

              {phase === "minute" &&
                Array.from({ length: 60 }, (_, val) => {
                  if (val % 5 === 0) return null;
                  const p = pos(val, 60, outerR);
                  const exact = val === minute;
                  return (
                    <circle
                      key={val}
                      cx={p.x}
                      cy={p.y}
                      r={exact ? 4 : 1.5}
                      className={`dial-tick ${exact ? "on" : ""}`}
                    />
                  );
                })}
            </svg>
          </div>

        </div>
      )}
    </div>
  );
}

function DatePicker({
  value,
  onChange,
}: {
  value: string;
  onChange: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const today = new Date();
  const parse = (s: string) => {
    const [y, mo, d] = s.split("-").map((x) => parseInt(x, 10));
    if (!y || !mo || !d) return null;
    return new Date(y, mo - 1, d);
  };
  const selected = parse(value) ?? today;
  const [viewY, setViewY] = useState(selected.getFullYear());
  const [viewM, setViewM] = useState(selected.getMonth());

  useEffect(() => {
    if (!open) return;
    const sel = parse(value) ?? new Date();
    setViewY(sel.getFullYear());
    setViewM(sel.getMonth());
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open, value]);

  const first = new Date(viewY, viewM, 1);
  const startDay = (first.getDay() + 6) % 7; // Monday-first
  const daysInMonth = new Date(viewY, viewM + 1, 0).getDate();
  const cells: (number | null)[] = [];
  for (let i = 0; i < startDay; i++) cells.push(null);
  for (let d = 1; d <= daysInMonth; d++) cells.push(d);
  // Always 6 rows (42 cells) for a fixed calendar size.
  while (cells.length < 42) cells.push(null);

  const monthLabel = new Date(viewY, viewM, 1).toLocaleString(undefined, {
    month: "long",
    year: "numeric",
  });

  const pick = (d: number) => {
    onChange(`${viewY}-${pad2(viewM + 1)}-${pad2(d)}`);
    setOpen(false);
  };

  const prev = () => {
    if (viewM === 0) {
      setViewM(11);
      setViewY((y) => y - 1);
    } else setViewM((m) => m - 1);
  };
  const next = () => {
    if (viewM === 11) {
      setViewM(0);
      setViewY((y) => y + 1);
    } else setViewM((m) => m + 1);
  };

  const isSel = (d: number) =>
    selected.getFullYear() === viewY &&
    selected.getMonth() === viewM &&
    selected.getDate() === d;

  return (
    <div className="picker" ref={ref}>
      <button type="button" className="picker-field" onClick={() => setOpen((o) => !o)}>
        <span className="picker-value">{value || "YYYY-MM-DD"}</span>
        <span className="picker-icon" aria-hidden>📅</span>
      </button>
      {open && (
        <div className="picker-pop date-pop">
          <div className="date-head">
            <button type="button" className="btn small date-nav" onClick={prev}>‹</button>
            <span className="date-month">{monthLabel}</span>
            <button type="button" className="btn small date-nav" onClick={next}>›</button>
          </div>
          <div className="date-grid date-dow">
            {["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].map((d) => (
              <span key={d}>{d}</span>
            ))}
          </div>
          <div className="date-grid date-days">
            {cells.map((d, i) =>
              d ? (
                <button
                  key={i}
                  type="button"
                  className={`date-cell ${isSel(d) ? "on" : ""}`}
                  onClick={() => pick(d)}
                >
                  {d}
                </button>
              ) : (
                <span key={i} className="date-cell empty" />
              ),
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function fmtTime(ts: number): string {
  return new Date(ts).toLocaleTimeString([], {
    hour12: false,
    fractionalSecondDigits: 3,
  });
}

// ---------------------------------------------------------------------------
// Add / edit form (popup)
// ---------------------------------------------------------------------------

interface EntryFormProps {
  /** null = add mode, otherwise the entry being edited */
  entry: Entry | null;
  onSubmit: (
    values: {
      label: string;
      dir: string;
      command: string;
      trigger: Trigger;
      alerts: Alerts;
      errorPolicy: ErrorPolicy;
      errorPopup: boolean;
      errorReruns: number;
      rerunDelay: number;
      rerunBackoff: boolean;
    },
    startNow: boolean,
  ) => void;
  onCancel: () => void;
}

function EntryForm({ entry, onSubmit, onCancel }: EntryFormProps) {
  const [label, setLabel] = useState(entry?.label ?? "");
  const [dir, setDir] = useState(entry?.dir ?? "");
  const [command, setCommand] = useState(entry?.command ?? "");
  const [startNow, setStartNow] = useState(true);
  const firstField = useRef<HTMLInputElement>(null);

  // --- trigger (run) ---
  const initial = entry?.trigger;
  const [mode, setMode] = useState<"startup" | "schedule">(
    initial && initial.kind !== "startup" ? "schedule" : "startup",
  );
  const [freq, setFreq] = useState<"once" | "daily" | "weekly">(
    initial && initial.kind !== "startup" ? initial.kind : "daily",
  );
  const [date, setDate] = useState(
    initial?.kind === "once" ? initial.date : new Date().toISOString().slice(0, 10),
  );
  const [time, setTime] = useState(
    initial && "time" in initial ? initial.time : "09:00",
  );
  const [days, setDays] = useState<number[]>(
    initial?.kind === "weekly" ? initial.days : [0, 2, 4],
  );

  // --- tone (alerts) ---
  const [alertStart, setAlertStart] = useState(entry?.alerts.start ?? false);
  const [startSound, setStartSound] = useState(
    entry?.alerts.startSound ?? "chime",
  );
  const [alertExit, setAlertExit] = useState(entry?.alerts.exit ?? false);
  const [exitSound, setExitSound] = useState(
    entry?.alerts.exitSound ?? "chime",
  );
  const [customSounds, setCustomSounds] = useState<CustomSound[]>(
    entry?.alerts.sounds ?? [],
  );
  const [errorSound, setErrorSound] = useState(entry?.alerts.errorSound ?? "");
  const [testPlaying, setTestPlaying] = useState<
    "start" | "exit" | "error" | null
  >(null);

  // While audio plays, the row's Test button becomes a Stop button.
  useEffect(
    () =>
      onPlaybackChange((playing) => {
        if (!playing) setTestPlaying(null);
      }),
    [],
  );

  const bumpReruns = (delta: number) => {
    setErrorReruns((prev) => {
      const next = (prev < 1 ? 0 : prev) + delta;
      return next < 1 ? -1 : next;
    });
  };

  const testButton = (row: "start" | "exit" | "error", key: string) => (
    <button
      type="button"
      className="btn small"
      onClick={() => {
        if (testPlaying === row) {
          stopSound();
        } else {
          playSound(key, customSounds);
          setTestPlaying(row);
        }
      }}
    >
      {testPlaying === row ? "■ Stop" : "▶ Test"}
    </button>
  );

  // --- on-error policy ---
  const [errorPolicy, setErrorPolicy] = useState<ErrorPolicy>(
    entry?.errorPolicy ?? "ignore",
  );
  const [errorPopup, setErrorPopup] = useState(entry?.errorPopup ?? false);
  const [errorReruns, setErrorReruns] = useState(entry?.errorReruns ?? -1);
  const [rerunDelay, setRerunDelay] = useState(entry?.rerunDelay ?? 3);
  const [rerunBackoff, setRerunBackoff] = useState(
    entry?.rerunBackoff ?? false,
  );
  const [formView, setFormView] = useState<"main" | "error">("main");

  const importSound = (forEvent: "start" | "exit" | "error") => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "audio/*";
    input.onchange = async () => {
      const file = input.files?.[0];
      if (!file) return;
      if (file.size > 1_000_000) {
        window.alert("Please pick a sound file under 1 MB.");
        return;
      }
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
      });
      const custom: CustomSound = {
        id: crypto.randomUUID(),
        name: file.name.replace(/\.[^.]+$/, ""),
        data,
      };
      setCustomSounds((prev) => [...prev, custom]);
      const key = `custom:${custom.id}`;
      if (forEvent === "start") {
        setStartSound(key);
        setAlertStart(true);
      } else if (forEvent === "exit") {
        setExitSound(key);
        setAlertExit(true);
      } else {
        setErrorSound(key);
      }
    };
    input.click();
  };

  const soundSelect = (
    value: string,
    onChange: (v: string) => void,
    withSameAsExit = false,
  ) => (
    <select value={value} onChange={(e) => onChange(e.target.value)}>
      {withSameAsExit && <option value="">Same as exit tone</option>}
      <optgroup label="Positive">
        {TONE_LIST.filter((t) => t.group === "positive").map((t) => (
          <option key={t.key} value={t.key}>
            {t.label}
          </option>
        ))}
      </optgroup>
      <optgroup label="Negative">
        {TONE_LIST.filter((t) => t.group === "negative").map((t) => (
          <option key={t.key} value={t.key}>
            {t.label}
          </option>
        ))}
      </optgroup>
      {customSounds.length > 0 && (
        <optgroup label="Imported">
          {customSounds.map((s) => (
            <option key={s.id} value={`custom:${s.id}`}>
              {s.name}
            </option>
          ))}
        </optgroup>
      )}
    </select>
  );

  const buildTrigger = (): Trigger | null => {
    if (mode === "startup") return { kind: "startup" };
    if (!time) return null;
    if (freq === "once") return date ? { kind: "once", date, time } : null;
    if (freq === "daily") return { kind: "daily", time };
    return days.length
      ? { kind: "weekly", days: [...days].sort((a, b) => a - b), time }
      : null;
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    firstField.current?.focus();
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  const browse = async () => {
    const picked = await api.browseDirectory(dir || undefined);
    if (picked) setDir(picked);
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onCancel();
      }}
    >
      <form
        className="modal"
        onSubmit={(e) => {
          e.preventDefault();
          const trigger = buildTrigger();
          if (!command.trim() || !trigger) return;
          onSubmit(
            {
              label,
              dir,
              command,
              trigger,
              alerts: {
                start: alertStart,
                startSound,
                exit: alertExit,
                exitSound,
                errorSound,
                sounds: customSounds,
              },
              errorPolicy,
              errorPopup,
              errorReruns,
              rerunDelay,
              rerunBackoff,
            },
            entry ? false : startNow,
          );
        }}
      >
        <div className="modal-body">
          <h2>{entry ? "Edit process" : "Add process"}</h2>

        {formView === "main" && (
          <>
        <label className="modal-field">
          <span>Label (optional)</span>
          <input
            ref={firstField}
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. dev server"
          />
        </label>

        <label className="modal-field">
          <span>Working directory (optional)</span>
          <div className="modal-row">
            <input
              value={dir}
              onChange={(e) => setDir(e.target.value)}
              placeholder="empty = your home directory"
            />
            <button type="button" className="btn small" onClick={browse}>
              Browse…
            </button>
          </div>
        </label>

        <label className="modal-field">
          <span>Command</span>
          <input
            className="mono"
            value={command}
            onChange={(e) => setCommand(e.target.value)}
            placeholder="e.g. npm run dev"
          />
        </label>

        <fieldset className="trigger-box">
          <legend>Run</legend>
          <div className="trigger-modes">
            <label className={`radio ${mode === "startup" ? "on" : ""}`}>
              <input
                type="radio"
                name="trigger-mode"
                checked={mode === "startup"}
                onChange={() => setMode("startup")}
              />
              When the app starts
            </label>
            <label className={`radio ${mode === "schedule" ? "on" : ""}`}>
              <input
                type="radio"
                name="trigger-mode"
                checked={mode === "schedule"}
                onChange={() => setMode("schedule")}
              />
              On a schedule
            </label>
          </div>

          {mode === "schedule" && (
            <div className="trigger-sched">
              <select
                value={freq}
                onChange={(e) => setFreq(e.target.value as "once" | "daily" | "weekly")}
              >
                <option value="once">Once</option>
                <option value="daily">Every day</option>
                <option value="weekly">On selected days</option>
              </select>

              {freq === "once" && (
                <div className="trigger-row">
                  <DatePicker value={date} onChange={setDate} />
                  <TimePicker value={time} onChange={setTime} />
                </div>
              )}

              {freq === "daily" && (
                <div className="trigger-row">
                  <span className="muted">at</span>
                  <TimePicker value={time} onChange={setTime} />
                </div>
              )}

              {freq === "weekly" && (
                <div className="trigger-row">
                  <div className="day-row">
                    {DAY_NAMES.map((d, i) => (
                      <button
                        key={d}
                        type="button"
                        title={d}
                        className={`day ${days.includes(i) ? "on" : ""}`}
                        onClick={() =>
                          setDays((prev) =>
                            prev.includes(i)
                              ? prev.filter((x) => x !== i)
                              : [...prev, i],
                          )
                        }
                      >
                        {d[0]}
                      </button>
                    ))}
                  </div>
                  <span className="muted">at</span>
                  <TimePicker value={time} onChange={setTime} />
                </div>
              )}
            </div>
          )}
        </fieldset>

        <fieldset className="trigger-box">
          <legend>Tone</legend>
          <div className="tone-row">
            <label className={`radio ${alertStart ? "on" : ""}`}>
              <input
                type="checkbox"
                checked={alertStart}
                onChange={(e) => setAlertStart(e.target.checked)}
              />
              when it starts by itself
            </label>
            <div className="trigger-row">
              {soundSelect(startSound, setStartSound)}
              <button
                type="button"
                className="btn small"
                onClick={() => importSound("start")}
              >
                Import…
              </button>
              {testButton("start", startSound)}
            </div>
          </div>
          <div className="tone-row">
            <label className={`radio ${alertExit ? "on" : ""}`}>
              <input
                type="checkbox"
                checked={alertExit}
                onChange={(e) => setAlertExit(e.target.checked)}
              />
              when it exits by itself
            </label>
            <div className="trigger-row">
              {soundSelect(exitSound, setExitSound)}
              <button
                type="button"
                className="btn small"
                onClick={() => importSound("exit")}
              >
                Import…
              </button>
              {testButton("exit", exitSound)}
            </div>
          </div>
        </fieldset>

        {!entry && (
          <label className="check modal-check">
            <input
              type="checkbox"
              checked={startNow}
              onChange={(e) => setStartNow(e.target.checked)}
            />
            start right away
          </label>
        )}
          </>
        )}

        {formView === "error" && (
          <fieldset className="trigger-box">
            <legend>When it exits with an error</legend>
            <div className="segmented">
              <div
                className="segmented-pill"
                style={{
                  transform: `translateX(${
                    (["ignore", "stopRuns", "rerun"] as const).indexOf(
                      errorPolicy,
                    ) * 100
                  }%)`,
                }}
              />
              {(
                [
                  ["ignore", "Default"],
                  ["stopRuns", "Stop future runs"],
                  ["rerun", "Rerun"],
                ] as const
              ).map(([key, label]) => (
                <button
                  key={key}
                  type="button"
                  className={`segment ${errorPolicy === key ? "on" : ""}`}
                  onClick={() => setErrorPolicy(key)}
                >
                  {label}
                </button>
              ))}
            </div>

            {errorPolicy === "rerun" && (
              <div className="trigger-row">
                <span className="muted">times</span>
                <div className="stepper">
                  <input
                    type="number"
                    min="1"
                    step="1"
                    value={errorReruns < 1 ? "" : String(errorReruns)}
                    onChange={(e) => {
                      const v = e.target.valueAsNumber;
                      setErrorReruns(
                        Number.isNaN(v) || v < 1 ? -1 : Math.floor(v),
                      );
                    }}
                  />
                  {errorReruns < 1 && (
                    <span className="stepper-inf" title="Infinite">
                      <InfinityIcon className="icon-lg" />
                    </span>
                  )}
                  <div className="stepper-btns">
                    <button
                      type="button"
                      className="step-btn"
                      aria-label="Increase times"
                      onClick={() => bumpReruns(1)}
                    >
                      <ChevronUpIcon className="icon-sm" />
                    </button>
                    <button
                      type="button"
                      className="step-btn"
                      aria-label="Decrease times"
                      onClick={() => bumpReruns(-1)}
                    >
                      <ChevronDownIcon className="icon-sm" />
                    </button>
                  </div>
                </div>
                {errorReruns >= 1 && (
                  <button
                    type="button"
                    className="btn small inf-btn"
                    title="Back to infinite"
                    onClick={() => setErrorReruns(-1)}
                  >
                    <InfinityIcon className="icon-lg" />
                  </button>
                )}
              </div>
            )}

            {errorPolicy === "rerun" && (
              <div className="trigger-row">
                <span className="muted">delay</span>
                <div className="stepper">
                  <input
                    type="number"
                    min="0"
                    step="1"
                    value={String(rerunDelay)}
                    onChange={(e) => {
                      const v = e.target.valueAsNumber;
                      setRerunDelay(
                        Number.isNaN(v) ? 0 : Math.max(0, Math.floor(v)),
                      );
                    }}
                  />
                  <div className="stepper-btns">
                    <button
                      type="button"
                      className="step-btn"
                      aria-label="Increase delay"
                      onClick={() => setRerunDelay((v) => Math.min(600, v + 1))}
                    >
                      <ChevronUpIcon className="icon-sm" />
                    </button>
                    <button
                      type="button"
                      className="step-btn"
                      aria-label="Decrease delay"
                      onClick={() => setRerunDelay((v) => Math.max(0, v - 1))}
                    >
                      <ChevronDownIcon className="icon-sm" />
                    </button>
                  </div>
                </div>
                <span className="muted">sec</span>
                <label className={`radio ${rerunBackoff ? "on" : ""}`}>
                  <input
                    type="checkbox"
                    checked={rerunBackoff}
                    onChange={(e) => setRerunBackoff(e.target.checked)}
                  />
                  grow exponentially
                </label>
              </div>
            )}

            <div className="trigger-row">
              <span className="muted">tone</span>
              {soundSelect(errorSound, setErrorSound, true)}
              <button
                type="button"
                className="btn small"
                onClick={() => importSound("error")}
              >
                Import…
              </button>
              {testButton("error", errorSound || exitSound)}
            </div>

            <label className={`radio ${errorPopup ? "on" : ""}`}>
              <input
                type="checkbox"
                checked={errorPopup}
                onChange={(e) => setErrorPopup(e.target.checked)}
              />
              show a popup about the failure
            </label>
          </fieldset>
        )}
        </div>

        <div className="modal-actions">
          <button
            type="button"
            className={`btn ${formView === "error" ? "primary" : ""}`}
            title="What should happen when this process exits with an error?"
            onClick={() => setFormView(formView === "error" ? "main" : "error")}
            aria-label={formView === "error" ? "Back" : "On error"}
          >
            {formView === "error" ? (
              <BackIcon className="icon" />
            ) : (
              <>
                <ShieldIcon className="icon" />
                On error
              </>
            )}
          </button>
          <div className="modal-actions-right">
            <button type="button" className="btn" onClick={onCancel}>
              Cancel
            </button>
            <button
              type="submit"
              className="btn primary"
              disabled={!command.trim()}
            >
              {entry ? "Save" : "Add"}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Alias form
// ---------------------------------------------------------------------------

interface AliasFormProps {
  alias: Alias | null;
  commandName: string;
  onSubmit: (values: { name: string; commands: AliasCommand[]; dir: string }) => Promise<void> | void;
  onCancel: () => void;
}

function AliasForm({ alias, commandName, onSubmit, onCancel }: AliasFormProps) {
  const [name, setName] = useState(alias?.name ?? "");
  const [dir, setDir] = useState(alias?.dir ?? "");
  const [commands, setCommands] = useState<AliasCommand[]>(
    alias?.commands?.length
      ? alias.commands.map((c) => ({ command: c.command, acceptsArgs: !!c.acceptsArgs }))
      : [{ command: "", acceptsArgs: true }],
  );
  const [err, setErr] = useState<string | null>(null);
  const cmd = commandName.trim() || "osu";
  const preview = `${cmd} ${name.trim() || "<alias>"} [args...]`;

  const setCmd = (i: number, patch: Partial<AliasCommand>) => {
    setCommands((prev) => prev.map((c, idx) => (idx === i ? { ...c, ...patch } : c)));
  };
  const addCmd = () =>
    setCommands((prev) => [...prev, { command: "", acceptsArgs: true }]);
  const removeCmd = (i: number) =>
    setCommands((prev) => (prev.length <= 1 ? prev : prev.filter((_, idx) => idx !== i)));
  const moveCmd = (i: number, dirDelta: -1 | 1) => {
    setCommands((prev) => {
      const j = i + dirDelta;
      if (j < 0 || j >= prev.length) return prev;
      const next = prev.slice();
      const [item] = next.splice(i, 1);
      next.splice(j, 0, item);
      return next;
    });
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr(null);
    const cleaned = commands
      .map((c) => ({ command: c.command.trim(), acceptsArgs: !!c.acceptsArgs }))
      .filter((c) => c.command.length > 0);
    if (!name.trim()) return setErr("Alias name is required.");
    if (cleaned.length === 0) return setErr("At least one command is required.");
    if (!cleaned.some((c) => c.acceptsArgs)) cleaned[cleaned.length - 1].acceptsArgs = true;
    try {
      await onSubmit({ name: name.trim(), commands: cleaned, dir: dir.trim() });
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={onCancel}>
      <form
        className="modal alias-form-modal"
        onMouseDown={(e) => e.stopPropagation()}
        onSubmit={submit}
      >
        <h2>{alias ? "Edit alias" : "Add alias"}</h2>
        <div className="modal-body">
          <label className="modal-field">
            <span>Alias name</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. dev"
              autoFocus
              spellCheck={false}
            />
            <div className="modal-hint">
              Terminal command: <code>{preview}</code>
            </div>
          </label>

          <div className="modal-field">
            <span>Commands (run in order)</span>
            <div className="alias-cmd-list">
              {commands.map((c, i) => (
                <div key={i} className="alias-cmd-item">
                  <div className="alias-cmd-item-head">
                    <span className="muted">#{i + 1}</span>
                    <label className="check alias-args-check">
                      <input
                        type="checkbox"
                        checked={c.acceptsArgs}
                        onChange={(e) => setCmd(i, { acceptsArgs: e.target.checked })}
                      />
                      gets extra args
                    </label>
                    <div className="alias-cmd-item-actions">
                      <button type="button" className="icon-btn" title="Move up" onClick={() => moveCmd(i, -1)} disabled={i === 0}>
                        <ChevronUpIcon className="icon-sm" />
                      </button>
                      <button type="button" className="icon-btn" title="Move down" onClick={() => moveCmd(i, 1)} disabled={i === commands.length - 1}>
                        <ChevronDownIcon className="icon-sm" />
                      </button>
                      <button type="button" className="icon-btn" title="Remove command" onClick={() => removeCmd(i)} disabled={commands.length <= 1}>
                        ✕
                      </button>
                    </div>
                  </div>
                  <input
                    className="mono"
                    value={c.command}
                    onChange={(e) => setCmd(i, { command: e.target.value })}
                    placeholder="e.g. npm run dev"
                    spellCheck={false}
                  />
                </div>
              ))}
            </div>
            <div className="alias-cmd-list-foot">
              <button type="button" className="btn small" onClick={addCmd}>＋ Add command</button>
            </div>
          </div>

          <label className="modal-field">
            <span>Working directory (optional)</span>
            <div className="modal-row">
              <input
                className="mono"
                value={dir}
                onChange={(e) => setDir(e.target.value)}
                placeholder="Leave empty to use current terminal directory"
                spellCheck={false}
              />
              <button
                type="button"
                className="btn"
                onClick={async () => {
                  const picked = await api.browseDirectory(dir || undefined);
                  if (picked) setDir(picked);
                }}
              >
                Browse…
              </button>
            </div>
          </label>
          {err && <div className="error-bar" style={{ marginTop: 4 }}>{err}</div>}
        </div>

        <div className="modal-actions">
          <span className="muted" style={{ fontSize: 11 }}>
            {alias ? "Updates the saved alias." : "Creates a reusable terminal alias."}
          </span>
          <div className="modal-actions-right">
            <button type="button" className="btn" onClick={onCancel}>
              Cancel
            </button>
            <button type="submit" className="btn primary" disabled={!name.trim() || !commands.some((c) => c.command.trim())}>
              {alias ? "Save changes" : "Add alias"}
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Process card
// ---------------------------------------------------------------------------

interface EntryCardProps {
  entry: Entry;
  running: boolean;
  errored: boolean;
  selected: boolean;
  onSelect: () => void;
  onStart: () => void;
  onStop: () => void;
  onDuplicate: () => void;
  onEdit: () => void;
  onRemove: () => void;
}

function EntryCard(props: EntryCardProps) {
  const { entry, running, errored, selected } = props;
  return (
    <div
      className={`entry ${selected ? "selected" : ""}`}
      onClick={props.onSelect}
    >
      <div className="entry-head">
        <span
          className={`dot ${running ? "dot-on" : errored ? "dot-err" : "dot-off"}`}
          title={running ? "running" : errored ? "exited with error" : "stopped"}
        />
        <div className="entry-label">
          {entry.label || entry.command || "untitled"}
        </div>
        <button
          className="icon-btn"
          title="Duplicate entry"
          onClick={(e) => {
            e.stopPropagation();
            props.onDuplicate();
          }}
        >
          <DuplicateIcon className="icon-sm" />
        </button>
        <button
          className="icon-btn"
          title="Edit entry"
          onClick={(e) => {
            e.stopPropagation();
            props.onEdit();
          }}
        >
          ✎
        </button>
        <button
          className="icon-btn"
          title="Remove entry"
          onClick={(e) => {
            e.stopPropagation();
            props.onRemove();
          }}
        >
          ✕
        </button>
      </div>
      {entry.dir && (
        <div className="entry-field entry-dir" title={entry.dir}>
          <span className="entry-dir-text">📁 {entry.dir}</span>
          <button
            type="button"
            className="icon-btn entry-dir-open"
            title="Open directory in file manager"
            aria-label="Open directory in file manager"
            onClick={(e) => {
              e.stopPropagation();
              void api.openInFileManager(entry.dir).catch((err) => {
                console.error("failed to open directory", err);
              });
            }}
          >
            <ArrowUpRightIcon className="icon-sm" />
          </button>
        </div>
      )}
      <div className="entry-field entry-cmd" title={entry.command}>
        ▶ {entry.command}
      </div>
      <div className="entry-foot">
        <span className="entry-trigger" title="Trigger">
          ⏱ {triggerLabel(entry.trigger)}
        </span>
        {entry.errorPolicy !== "ignore" && (
          <span
            className="entry-policy"
            title={policyLabel(entry.errorPolicy)}
          >
            <ShieldIcon className="icon-sm" />
          </span>
        )}
        <span
          className={`entry-status ${
            running ? "on" : entry.halted ? "warn" : errored ? "err" : ""
          }`}
        >
          {running
            ? "running"
            : entry.halted
              ? "halted"
              : errored
                ? "failed"
                : "stopped"}
        </span>
        <button
          className={`btn small ${running ? "danger" : "primary"}`}
          onClick={(e) => {
            e.stopPropagation();
            if (running) props.onStop();
            else props.onStart();
          }}
        >
          {running ? "Stop" : "Start"}
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

type FormState =
  | { mode: "add"; prefill?: Entry }
  | { mode: "edit"; entry: Entry }
  | null;

function getViewParams(): URLSearchParams {
  const search = new URLSearchParams(window.location.search);
  if (search.get("view")) return search;
  const hash = window.location.hash.replace(/^#/, "");
  return new URLSearchParams(hash);
}

function isErrorPopupWindow(): boolean {
  return (
    (window as unknown as { __ERROR_POPUP__?: boolean }).__ERROR_POPUP__ === true ||
    getViewParams().get("view") === "error-popup"
  );
}

export default function App() {
  if (isErrorPopupWindow()) {
    return <ErrorPopupView />;
  }
  return <MainApp />;
}

function MainApp() {
  const [entries, setEntries] = useState<Entry[]>([]);
  const [aliases, setAliases] = useState<Alias[]>([]);
  const [tab, setTab] = useState<"scheduled" | "alias">("scheduled");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [logs, setLogs] = useState<Record<string, LogLine[]>>({});
  const [running, setRunning] = useState<Record<string, boolean>>({});
  const [exits, setExits] = useState<Record<string, number | null>>({});
  const [settings, setSettings] = useState<Settings>({ autostart: true, aliasActive: false, aliasCommand: "osu" });
  const [autoScroll, setAutoScroll] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [form, setForm] = useState<FormState>(null);

  // Lines arrive fast; buffer them and flush on a timer instead of one
  // re-render per line.
  const pending = useRef<Record<string, LogLine[]>>({});
  const lastSeq = useRef<Record<string, number>>({});
  const scrollRef = useRef<HTMLDivElement>(null);
  const entriesRef = useRef<Entry[]>([]);
  entriesRef.current = entries;
  const logsRef = useRef<Record<string, LogLine[]>>({});
  logsRef.current = logs;

  useEffect(() => {
    let disposed = false;
    let es: EventSource | null = null;

    // Reload everything from the service. Called on every SSE (re)connect, so
    // the UI realigns after a service restart or changes made elsewhere.
    const refresh = async () => {
      try {
        const state: ServerState = await api.getState();
        if (disposed) return;
        for (const [id, lines] of Object.entries(state.logs)) {
          lastSeq.current[id] = lines.length ? lines[lines.length - 1].seq : 0;
        }
        setEntries(state.entries);
        setAliases(state.aliases ?? []);
        setLogs(state.logs);
        setRunning(state.running);
        setExits(state.exits ?? {});
        const nextAliasCmd =
          typeof state.settings?.aliasCommand === "string" && state.settings.aliasCommand.trim()
            ? state.settings.aliasCommand
            : "osu";
        setSettings({
          autostart: !!state.settings?.autostart,
          aliasActive: !!state.settings?.aliasActive,
          aliasCommand: nextAliasCmd,
        });
        setAliasCmdDraft(nextAliasCmd);
        setSelectedId((prev) => prev ?? state.entries[0]?.id ?? null);
        setError(null);
      } catch (e) {
        if (!disposed) setError(String(e));
      }
    };

    const onStreamEvent = (ev: StreamEvent) => {
      if (ev.type === "hello") {
        void refresh();
      } else if (ev.type === "log") {
        (pending.current[ev.id] ??= []).push(ev.line);
      } else if (ev.type === "status") {
        setRunning((prev) => ({ ...prev, [ev.id]: ev.running }));
        setExits((prev) => {
          const next = { ...prev };
          if (!ev.running && !ev.expected && ev.exitCode !== 0) {
            // Unexpected, non-zero exit: red dot.
            next[ev.id] = ev.exitCode;
          } else {
            delete next[ev.id];
          }
          return next;
        });
        // A fresh start: clear the console so output begins from the top.
        if (ev.running) {
          pending.current[ev.id] = [];
          lastSeq.current[ev.id] = 0;
          setLogs((prev) => ({ ...prev, [ev.id]: [] }));
        }
        // Opt-in tones: only for the process's own lifecycle events —
        // starting by itself (boot/schedule) and exiting by itself (not
        // stopped by the user).
        const entry = entriesRef.current.find((e) => e.id === ev.id);
        if (entry) {
          if (ev.running) {
            if (ev.auto && entry.alerts.start) {
              playSound(entry.alerts.startSound, entry.alerts.sounds);
            }
          } else {
            const error = !ev.expected && ev.exitCode !== 0;
            if (!ev.expected && entry.alerts.exit) {
              // On error exits the override sound plays instead of the
              // regular exit tone.
              const sound =
                error && entry.alerts.errorSound
                  ? entry.alerts.errorSound
                  : entry.alerts.exitSound;
              playSound(sound, entry.alerts.sounds);
            }
            if (error && entry.errorPopup) {
              const name = entry.label || entry.command || "process";
              const all = [
                ...(logsRef.current[ev.id] ?? []),
                ...(pending.current[ev.id] ?? []),
              ];
              const errLines = all
                .filter((l) => l.stream === "stderr")
                .slice(-5)
                .map((l) => l.text);
              const code = ev.exitCode != null ? ` (code ${ev.exitCode})` : "";
              sendNotification({
                title: "On Start Up — process failed",
                body:
                  `${name} exited with error${code}.` +
                  (errLines.length ? `\n${errLines[errLines.length - 1]}` : ""),
              });
              // Open an independent popup window (works even when the main
              // window is hidden to the tray).
              invoke("show_error_popup", {
                id: ev.id,
                name,
                exitCode: ev.exitCode ?? null,
                lines: errLines.join("\n"),
              }).catch(() => {});
            }
          }
        }
      }
    };

    // Audio is locked until the first user gesture.
    window.addEventListener("pointerdown", unlockAudio, { once: true });

    (async () => {
      try {
        const info = await api.serverInfo();
        api.init(info.url);
        const ensure = await api.ensureServer();
        if (!ensure.running) {
          throw new Error(ensure.error ?? "background service is not running");
        }
        if (disposed) return;
        es = api.openStream(onStreamEvent);
      } catch (e) {
        if (!disposed) setError(String(e));
      }
    })();

    return () => {
      disposed = true;
      window.removeEventListener("pointerdown", unlockAudio);
      es?.close();
    };
  }, []);

  // Flush buffered log lines into state.
  useEffect(() => {
    const timer = setInterval(() => {
      const batch = pending.current;
      if (!Object.keys(batch).length) return;
      pending.current = {};
      setLogs((prev) => {
        const next = { ...prev };
        for (const [id, lines] of Object.entries(batch)) {
          const fresh = lines.filter((l) => l.seq > (lastSeq.current[id] ?? 0));
          if (!fresh.length) continue;
          lastSeq.current[id] = fresh[fresh.length - 1].seq;
          const arr = [...(next[id] ?? []), ...fresh];
          if (arr.length > MAX_LINES) arr.splice(0, arr.length - MAX_LINES);
          next[id] = arr;
        }
        return next;
      });
    }, FLUSH_MS);
    return () => clearInterval(timer);
  }, []);

  const selected = useMemo(
    () => entries.find((e) => e.id === selectedId) ?? null,
    [entries, selectedId],
  );
  const lines = (selectedId && logs[selectedId]) || [];
  const lineCount = lines.length;

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [lineCount, autoScroll, selectedId]);

  const submitForm = async (
    values: {
      label: string;
      dir: string;
      command: string;
      trigger: Trigger;
      alerts: Alerts;
      errorPolicy: ErrorPolicy;
      errorPopup: boolean;
      errorReruns: number;
      rerunDelay: number;
      rerunBackoff: boolean;
    },
    startNow: boolean,
  ) => {
    try {
      if (form?.mode === "edit") {
        const updated: Entry = { ...form.entry, ...values };
        await api.updateEntry(updated);
        setEntries((prev) => prev.map((e) => (e.id === updated.id ? updated : e)));
      } else {
        const created = await api.addEntry(values);
        setEntries((prev) => [...prev, created]);
        setSelectedId(created.id);
        if (startNow) await api.startEntry(created.id);
      }
      setForm(null);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const start = async (id: string) => {
    try {
      setError(null);
      await api.startEntry(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const stop = async (id: string) => {
    try {
      setError(null);
      await api.stopEntry(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const startAll = async () => {
    setError(null);
    const errors = await api.startAll().catch((e) => [String(e)]);
    if (errors.length) setError(errors.join(" · "));
  };

  const remove = async (id: string) => {
    try {
      await api.removeEntry(id);
      setEntries((prev) => {
        const next = prev.filter((e) => e.id !== id);
        setSelectedId((cur) => (cur === id ? (next[0]?.id ?? null) : cur));
        return next;
      });
    } catch (e) {
      setError(String(e));
    }
  };

  const clearLogs = async () => {
    if (!selectedId) return;
    await api.clearLogs(selectedId).catch(() => {});
    setLogs((prev) => ({ ...prev, [selectedId]: [] }));
  };

  const copyLogs = () => {
    const text = lines.map((l) => `[${fmtTime(l.ts)}] ${l.text}`).join("\n");
    navigator.clipboard.writeText(text).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1200);
    });
  };

  const toggleAutostart = async () => {
    const next = { ...settings, autostart: !settings.autostart };
    setSettings(next);
    try {
      await api.setSettings(next);
    } catch (e) {
      setSettings(settings);
      setError(String(e));
    }
  };

  const toggleAliasActive = async () => {
    const next = { ...settings, aliasActive: !settings.aliasActive };
    setSettings(next);
    try {
      await api.setSettings(next);
    } catch (e) {
      setSettings(settings);
      setError(String(e));
    }
  };

  const saveAliasCommand = async () => {
    const normalized = aliasCmdDraft.trim() || "osu";
    if (normalized === (settings.aliasCommand || "osu")) {
      setAliasCmdDraft(normalized);
      return true;
    }
    const next = { ...settings, aliasCommand: normalized };
    setSettings(next);
    try {
      await api.setSettings(next);
      setAliasCmdDraft(normalized);
      return true;
    } catch (e) {
      setSettings(settings);
      throw e;
    }
  };

  const [aliasForm, setAliasForm] = useState<
    { mode: "add"; prefill?: Alias } | { mode: "edit"; alias: Alias } | null
  >(null);
  const [aliasCmdDraft, setAliasCmdDraft] = useState("osu");
  const [aliasCmdEditOpen, setAliasCmdEditOpen] = useState(false);
  const [aliasCmdEditErr, setAliasCmdEditErr] = useState<string | null>(null);

  const submitAlias = async (values: { name: string; commands: AliasCommand[]; dir: string }) => {
    if (aliasForm?.mode === "edit") {
      const updated: Alias = { ...aliasForm.alias, ...values };
      await api.updateAlias(updated);
      setAliases((prev) => prev.map((a) => (a.id === updated.id ? updated : a)));
    } else {
      const created = await api.addAlias(values);
      setAliases((prev) => [...prev, created]);
    }
    setAliasForm(null);
  };

  const removeAlias = async (id: string) => {
    try {
      await api.removeAlias(id);
      setAliases((prev) => prev.filter((a) => a.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  const runningCount = entries.filter((e) => running[e.id]).length;
  const aliasCmd = settings.aliasCommand?.trim() || "osu";

  return (
    <div className="app">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">▶</span>
          <h1>On Start Up</h1>
        </div>
        <div className="segmented cols-2 top-segmented" role="tablist" aria-label="Main tabs">
          <div
            className="segmented-pill"
            style={{
              transform: `translateX(${(["scheduled", "alias"] as const).indexOf(tab) * 100}%)`,
            }}
          />
          {(
            [
              ["scheduled", "Scheduled"],
              ["alias", "Alias"],
            ] as const
          ).map(([key, label]) => (
            <button
              key={key}
              type="button"
              role="tab"
              aria-selected={tab === key}
              className={`segment ${tab === key ? "on" : ""}`}
              onClick={() => setTab(key)}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="topbar-actions">
          <label
            className="check"
            title="Start the UI (and with it the background service) when you log in"
          >
            <input
              type="checkbox"
              checked={settings.autostart}
              onChange={toggleAutostart}
            />
            start with system
          </label>
          <label
            className="check"
            title={`Register the ${aliasCmd} command on your PATH so you can run aliases from any terminal`}
          >
            <input
              type="checkbox"
              checked={settings.aliasActive}
              onChange={toggleAliasActive}
            />
            active alias
          </label>
        </div>
      </header>

      {tab === "scheduled" ? (
        <main className="split">
          <section className="list-pane">
            <div className="pane-title">
              <span>Processes</span>
              <div className="pane-actions">
                <button className="btn small" onClick={() => api.stopAll().catch(() => {})}>
                  Stop all
                </button>
                <button className="btn small" onClick={startAll}>
                  Start all
                </button>
                <button
                  className="btn small primary"
                  onClick={() => setForm({ mode: "add" })}
                >
                  ＋ Add
                </button>
              </div>
            </div>
            <div className="entry-list">
              {entries.length === 0 && (
                <div className="empty">
                  No processes yet.
                  <br />
                  Click <strong>＋ Add</strong> to create one.
                  <br />
                  Every entry runs automatically on start.
                </div>
              )}
              {entries.map((entry) => (
                <EntryCard
                  key={entry.id}
                  entry={entry}
                  running={!!running[entry.id]}
                  errored={exits[entry.id] !== undefined}
                  selected={entry.id === selectedId}
                  onSelect={() => setSelectedId(entry.id)}
                  onStart={() => start(entry.id)}
                  onStop={() => stop(entry.id)}
                  onDuplicate={() => setForm({ mode: "add", prefill: entry })}
                  onEdit={() => setForm({ mode: "edit", entry })}
                  onRemove={() => remove(entry.id)}
                />
              ))}
            </div>
          </section>

          <section className="log-pane">
            <header className="log-head">
              <div className="log-title">
                {selected ? (
                  <>
                    <span
                      className={`dot ${
                        running[selected.id]
                          ? "dot-on"
                          : exits[selected.id] !== undefined
                            ? "dot-err"
                            : "dot-off"
                      }`}
                    />
                    <strong>
                      {selected.label || selected.command || "untitled"}
                    </strong>
                  </>
                ) : (
                  <span className="muted">
                    Select a process to see its output
                  </span>
                )}
              </div>
              <div className="log-actions">
                <label className="check">
                  <input
                    type="checkbox"
                    checked={autoScroll}
                    onChange={(e) => setAutoScroll(e.target.checked)}
                  />
                  auto-scroll
                </label>
                <button
                  className="btn small"
                  onClick={copyLogs}
                  disabled={!lines.length}
                >
                  {copied ? "Copied ✓" : "Copy"}
                </button>
                <button
                  className="btn small"
                  onClick={clearLogs}
                  disabled={!lines.length}
                >
                  Clear
                </button>
              </div>
            </header>
            <div className="log-body" ref={scrollRef}>
              {lineCount === 0 ? (
                <div className="log-empty">
                  stdout / stderr of the selected process appears here.
                </div>
              ) : (
                lines.map((l, i) => (
                  <div key={`${l.seq}-${i}`} className={`line line-${l.stream}`}>
                    <span className="line-num">{i + 1}</span>
                    <span className="line-text">{l.text || " "}</span>
                  </div>
                ))
              )}
            </div>
          </section>
        </main>
      ) : (
        <main className="alias-main">
          <section className="alias-pane">
            <div className="pane-title">
              <span>Command aliases</span>
              <div className="pane-actions">
                <span className="alias-cmd-chip" title="Current alias command name">
                  <code>{aliasCmd}</code>
                </span>
                <button
                  className="icon-btn"
                  title="Edit command name"
                  onClick={() => {
                    setAliasCmdDraft(aliasCmd);
                    setAliasCmdEditErr(null);
                    setAliasCmdEditOpen(true);
                  }}
                >
                  <PencilIcon className="icon-sm" />
                </button>
                <button
                  className="btn small primary"
                  onClick={() => setAliasForm({ mode: "add" })}
                >
                  ＋ Add alias
                </button>
              </div>
            </div>
            <div className="alias-toolbar">
              <span className="muted alias-hint">
                {settings.aliasActive
                  ? `Active: use in terminal as \`${aliasCmd} <alias> [args...]\``
                  : `Turn on “active alias” in the top bar to register \`${aliasCmd}\` on your PATH`}
              </span>
            </div>
            <div className="alias-list">
              {aliases.length === 0 && (
                <div className="empty">
                  No aliases yet.
                  <br />
                  Click <strong>＋ Add alias</strong> to create one.
                </div>
              )}
              {aliases.map((a) => (
                <div key={a.id} className="alias-card">
                  <div className="alias-card-main">
                    <div className="alias-name"><code>{aliasCmd} {a.name}</code></div>
                    <div className="alias-commands">
                      {a.commands.map((c, i) => (
                        <div key={i} className="alias-command-line">
                          <span className="alias-command-index">{i + 1}.</span>
                          <span className="alias-command">{c.command}</span>
                          {c.acceptsArgs && <span className="alias-args-badge">args</span>}
                        </div>
                      ))}
                    </div>
                    {a.dir && <div className="alias-dir muted">{a.dir}</div>}
                  </div>
                  <div className="alias-card-actions">
                    <button
                      className="icon-btn"
                      title="Duplicate alias"
                      onClick={() => setAliasForm({ mode: "add", prefill: a })}
                    >
                      <DuplicateIcon className="icon-sm" />
                    </button>
                    <button className="btn small" onClick={() => setAliasForm({ mode: "edit", alias: a })}>Edit</button>
                    <button className="btn small danger" onClick={() => removeAlias(a.id)}>Remove</button>
                  </div>
                </div>
              ))}
            </div>
          </section>
        </main>
      )}

      <footer className="statusbar">
        <span>
          {entries.length} entr{entries.length === 1 ? "y" : "es"} ·{" "}
          {runningCount} running
        </span>
      </footer>

      {form && (
        <EntryForm
          entry={form.mode === "edit" ? form.entry : form.prefill ?? null}
          onSubmit={submitForm}
          onCancel={() => setForm(null)}
        />
      )}
      {aliasForm && (
        <AliasForm
          alias={aliasForm.mode === "edit" ? aliasForm.alias : aliasForm.prefill ?? null}
          commandName={settings.aliasCommand || "osu"}
          onSubmit={submitAlias}
          onCancel={() => setAliasForm(null)}
        />
      )}
      {aliasCmdEditOpen && (
        <div className="modal-backdrop" onMouseDown={() => setAliasCmdEditOpen(false)}>
          <form
            className="modal"
            onMouseDown={(e) => e.stopPropagation()}
            onSubmit={async (e) => {
              e.preventDefault();
              setAliasCmdEditErr(null);
              try {
                if (await saveAliasCommand()) setAliasCmdEditOpen(false);
              } catch (e) {
                setAliasCmdEditErr(String(e));
              }
            }}
          >
            <h2>Edit command name</h2>
            <div className="modal-body">
              <label className="modal-field">
                <span>Command name</span>
                <input
                  className="mono"
                  value={aliasCmdDraft}
                  onChange={(e) => setAliasCmdDraft(e.target.value)}
                  placeholder="osu"
                  autoFocus
                  spellCheck={false}
                />
                <div className="modal-hint">
                  This name is registered on your PATH. Example: <code>{aliasCmdDraft.trim() || "osu"} &lt;alias&gt; [args...]</code>
                </div>
              </label>
              {aliasCmdEditErr && <div className="error-bar" style={{ marginTop: 4 }}>{aliasCmdEditErr}</div>}
            </div>
            <div className="modal-actions">
              <span />
              <div className="modal-actions-right">
                <button type="button" className="btn" onClick={() => setAliasCmdEditOpen(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn primary">
                  Save
                </button>
              </div>
            </div>
          </form>
        </div>
      )}
      {error && (
        <div className="modal-backdrop" onMouseDown={() => setError(null)}>
          <div className="modal" onMouseDown={(e) => e.stopPropagation()} role="alertdialog" aria-modal="true">
            <h2>Error</h2>
            <div className="modal-body">
              <div className="error-bar">{error}</div>
            </div>
            <div className="modal-actions">
              <span />
              <div className="modal-actions-right">
                <button type="button" className="btn primary" onClick={() => setError(null)}>
                  Dismiss
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
