import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * What Summo is costing right now, and what it is running.
 *
 * Every field is a *reading*. Nothing here is derived from what was configured — a model named in
 * settings and a model actually loaded are different claims, and a readout that showed the first
 * while saying "running" would be the most confident kind of wrong. Where the daemon cannot
 * measure something it sends `null`, never zero: a daemon that cannot be measured and one using
 * nothing must not draw the same.
 */

export interface Loaded {
  /** `live`, `refine`, `denoise`, or `warm` — the model held between recordings. */
  role: string;
  id: string;
}

export interface Perf {
  /** Resident memory of the daemon, megabytes. `null` where the process cannot see itself. */
  rss_mb: number | null;
  /** Share of one core, as a percentage. `null` on the first reading — a rate needs two samples. */
  cpu_percent: number | null;
  total_ram_mb: number;
  available_ram_mb: number;
  cores: number;
  recording: boolean;
  /** Seconds of audio accepted by the running session, or `null` when nothing is recording. */
  audio_s: number | null;
  segments: number | null;
  models: Loaded[];
  /** Background work that explains a busy daemon which is not recording. */
  busy: { imports: number; installs: number; dubs: number };
}

export async function perf(handshake: Handshake): Promise<Perf> {
  return readJson<Perf>(await fetch(url(handshake, "/perf")));
}

/**
 * How often to ask.
 *
 * Two seconds. It is also what makes `cpu_percent` mean anything — the daemon computes it from the
 * gap between two readings, so the poll interval *is* the averaging window. Slower would report a
 * smoother number about a longer ago.
 */
export const POLL_MS = 2_000;

/**
 * Whether the readout is showing.
 *
 * `interface.show_performance` in the vault is the truth, so the choice survives a reload and
 * reaches a second window. This is a local mirror of it, for the same reason `theme.ts` keeps one:
 * reading the vault before the first paint would put a round trip in front of every pixel, and the
 * panel would flash on for anybody who had turned it off.
 *
 * Off is the default at every level. A permanent gauge in the corner of a recorder is a thing to
 * watch instead of the meeting.
 */
const STORAGE_KEY = "summo.perf";
const CHANGED = "summo:perf";

export function isOn(): boolean {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "on";
  } catch {
    return false;
  }
}

/**
 * Turn it on or off here and now, and tell every listener.
 *
 * The event is what makes the switch in settings and the panel in the corner the same switch —
 * without it the panel would appear on the next reload, which reads as the toggle not working.
 */
export function show(on: boolean): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, on ? "on" : "off");
  } catch {
    // Applied anyway: it works for this session, which beats not working at all.
  }
  window.dispatchEvent(new CustomEvent(CHANGED, { detail: on }));
}

/** Adopt what the vault says, without announcing it as a change the user just made. */
export function adopt(on: boolean): void {
  if (on === isOn()) return;
  show(on);
}

export function onChange(listener: (on: boolean) => void): () => void {
  const handle = (event: Event) => listener((event as CustomEvent<boolean>).detail);
  window.addEventListener(CHANGED, handle);
  return () => window.removeEventListener(CHANGED, handle);
}

/**
 * Summo's share of this machine's memory, as a percentage, or `null` when it cannot be worked out.
 *
 * Against the *total*, not against what is free. "Summo is using 40% of your free memory" says
 * something different every time another application opens, which makes it useless as a number to
 * watch — and alarming at exactly the moment it should not be.
 */
export function shareOfRam(reading: Perf): number | null {
  if (reading.rss_mb === null || reading.total_ram_mb <= 0) return null;
  return (reading.rss_mb / reading.total_ram_mb) * 100;
}

/**
 * How much background work is running, as one number.
 *
 * The readout's job when nothing is recording is to answer "why is this busy". Three separate
 * counters make the reader add them up; the panel names whichever ones are non-zero.
 */
export function busyCount(reading: Perf): number {
  return reading.busy.imports + reading.busy.installs + reading.busy.dubs;
}

/** The background jobs that are actually running, as `{ key, count }`, dropping the zeroes. */
export function busyParts(reading: Perf): { key: string; count: number }[] {
  return (
    [
      ["perf.busy_imports", reading.busy.imports],
      ["perf.busy_installs", reading.busy.installs],
      ["perf.busy_dubs", reading.busy.dubs],
    ] as const
  )
    .filter(([, count]) => count > 0)
    .map(([key, count]) => ({ key, count }));
}
