// Synthesized alert tones — no sound files needed. Sounds are opt-in per
// entry and only fire for a process's own lifecycle events (starting by
// itself, exiting by itself). Custom sounds can be imported per entry.
//
// Built-in tones come in two families: positive (starts, clean exits) and
// negative (failures).
//
// Playback is single-instance: starting a sound stops the previous one, so
// tests and alerts never stack. `onPlaybackChange` lets the UI turn a Test
// button into a Stop button while audio is playing.

import type { CustomSound } from "./types";

let ctx: AudioContext | null = null;

function audio(): AudioContext | null {
  try {
    if (!ctx) ctx = new AudioContext();
    if (ctx.state === "suspended") void ctx.resume();
    return ctx;
  } catch {
    return null;
  }
}

interface Note {
  freq: number;
  at: number;
  dur: number;
  type: OscillatorType;
  gain: number;
}

export interface ToneDef {
  label: string;
  group: "positive" | "negative";
  notes: Note[];
}

const TONES: Record<string, ToneDef> = {
  // --- positive ---
  chime: {
    label: "Chime",
    group: "positive",
    notes: [
      { freq: 660, at: 0, dur: 0.18, type: "triangle", gain: 0.2 },
      { freq: 880, at: 0.12, dur: 0.28, type: "triangle", gain: 0.2 },
    ],
  },
  ping: {
    label: "Ping",
    group: "positive",
    notes: [{ freq: 880, at: 0, dur: 0.16, type: "sine", gain: 0.22 }],
  },
  bell: {
    label: "Bell",
    group: "positive",
    notes: [
      { freq: 1320, at: 0, dur: 0.45, type: "sine", gain: 0.16 },
      { freq: 1980, at: 0, dur: 0.3, type: "sine", gain: 0.07 },
    ],
  },
  rise: {
    label: "Rise",
    group: "positive",
    notes: [
      { freq: 440, at: 0, dur: 0.12, type: "triangle", gain: 0.2 },
      { freq: 587, at: 0.1, dur: 0.12, type: "triangle", gain: 0.2 },
      { freq: 784, at: 0.2, dur: 0.24, type: "triangle", gain: 0.2 },
    ],
  },
  success: {
    label: "Success",
    group: "positive",
    notes: [
      { freq: 523, at: 0, dur: 0.14, type: "sine", gain: 0.2 },
      { freq: 659, at: 0.11, dur: 0.14, type: "sine", gain: 0.2 },
      { freq: 784, at: 0.22, dur: 0.32, type: "sine", gain: 0.2 },
    ],
  },

  // --- negative ---
  buzz: {
    label: "Buzz",
    group: "negative",
    notes: [{ freq: 220, at: 0, dur: 0.28, type: "square", gain: 0.1 }],
  },
  fall: {
    label: "Fall",
    group: "negative",
    notes: [
      { freq: 880, at: 0, dur: 0.14, type: "triangle", gain: 0.18 },
      { freq: 660, at: 0.12, dur: 0.14, type: "triangle", gain: 0.18 },
      { freq: 440, at: 0.24, dur: 0.26, type: "triangle", gain: 0.18 },
    ],
  },
  alert: {
    label: "Alert",
    group: "negative",
    notes: [
      { freq: 330, at: 0, dur: 0.12, type: "square", gain: 0.1 },
      { freq: 330, at: 0.18, dur: 0.18, type: "square", gain: 0.1 },
    ],
  },
  fail: {
    label: "Fail",
    group: "negative",
    notes: [
      { freq: 659, at: 0, dur: 0.18, type: "triangle", gain: 0.18 },
      { freq: 523, at: 0.15, dur: 0.18, type: "triangle", gain: 0.18 },
      { freq: 392, at: 0.3, dur: 0.34, type: "triangle", gain: 0.18 },
    ],
  },
};

/** Flat list for building selectors. */
export const TONE_LIST = Object.entries(TONES).map(([key, def]) => ({
  key,
  label: def.label,
  group: def.group,
}));

// --- single-instance playback ---

type StopFn = () => void;

let stopCurrent: StopFn | null = null;
let token = 0;
const listeners = new Set<(playing: boolean) => void>();

function notify() {
  const playing = stopCurrent !== null;
  for (const l of listeners) l(playing);
}

/** Stop whatever is playing (if anything). */
export function stopSound() {
  token += 1;
  const stop = stopCurrent;
  stopCurrent = null;
  stop?.();
  notify();
}

/** Called with true when playback starts and false when it ends or stops. */
export function onPlaybackChange(cb: (playing: boolean) => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function beginPlayback(
  makeSound: (finish: () => void) => { stop: StopFn; durationMs: number | null },
) {
  stopSound();
  const myToken = ++token;
  const finish = () => {
    if (token !== myToken) return;
    stopCurrent = null;
    notify();
  };
  const { stop, durationMs } = makeSound(finish);
  stopCurrent = stop;
  notify();
  if (durationMs != null) {
    window.setTimeout(finish, durationMs);
  }
}

function startTone(key: string) {
  const ac = audio();
  if (!ac) return;
  const notes = (TONES[key] ?? TONES.chime).notes;
  beginPlayback(() => {
    const oscs: OscillatorNode[] = [];
    let total = 0;
    for (const n of notes) {
      const osc = ac.createOscillator();
      const gain = ac.createGain();
      osc.type = n.type;
      osc.frequency.value = n.freq;
      const t0 = ac.currentTime + n.at;
      gain.gain.setValueAtTime(0.0001, t0);
      gain.gain.exponentialRampToValueAtTime(n.gain, t0 + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + n.dur);
      osc.connect(gain).connect(ac.destination);
      osc.start(t0);
      osc.stop(t0 + n.dur + 0.05);
      oscs.push(osc);
      total = Math.max(total, n.at + n.dur);
    }
    return {
      stop: () => {
        for (const o of oscs) {
          try {
            o.stop();
          } catch {
            /* already stopped */
          }
        }
      },
      durationMs: (total + 0.12) * 1000,
    };
  });
}

function startCustom(sound: CustomSound) {
  beginPlayback((finish) => {
    const el = new Audio(sound.data);
    el.volume = 1;
    el.onended = finish;
    void el.play().catch(() => {});
    return {
      stop: () => {
        el.pause();
        el.currentTime = 0;
      },
      durationMs: null,
    };
  });
}

/** Play a sound key: a built-in tone or "custom:<id>" from imported sounds. */
export function playSound(key: string, sounds: CustomSound[]) {
  if (key.startsWith("custom:")) {
    const sound = sounds.find((s) => `custom:${s.id}` === key);
    if (sound) startCustom(sound);
    return;
  }
  startTone(key);
}

/** Browsers keep audio locked until the first user gesture. */
export function unlockAudio() {
  audio();
}
