/**
 * Reading the vault from the app.
 *
 * The daemon does the scanning; this is the typed edge of it, plus the formatting that turns
 * `2026-08-09` into something a person reads without decoding. The formatting is separate from the
 * fetching on purpose — labels are where the Vietnamese-first decisions live, and they are worth
 * testing without a server.
 */

import type { Handshake } from "./engine";

export interface MeetingSummary {
  id: string;
  title: string;
  folder: string;
  /** ISO-8601 with the offset the meeting happened in. */
  date: string;
  /** `YYYY-MM-DD`, in that same offset. */
  day: string;
  duration: number;
  participants: string[];
  tags: string[];
  has_summary: boolean;
  size_bytes: number;
  file: string;
}

export interface SummaryGroup {
  key: string;
  meetings: MeetingSummary[];
}

export interface Stats {
  meetings: number;
  total_duration: number;
  people: number;
  tags: number;
  without_summary: number;
  last_seven_days: number;
  last_seven_days_duration: number;
  latest: string | null;
}

export interface LibraryView {
  groups: SummaryGroup[];
  total: number;
  stats: Stats;
  folders: string[];
  tags: { name: string; count: number }[];
  people: { name: string; count: number }[];
  skipped: { path: string; reason: string }[];
}

export interface Excerpt {
  text: string;
  t0: number | null;
  speaker: string | null;
}

export interface SearchHit {
  meeting: MeetingSummary;
  matches: number;
  excerpts: Excerpt[];
}

export interface Segment {
  seq: number;
  lane: string;
  text: string;
  t0: number;
  t1: number;
  speaker: string | null;
}

export interface MeetingDetail {
  summary: MeetingSummary;
  sections: { heading: string; body: string; draft: boolean }[];
  transcript: Segment[];
  audio: string[];
}

export type GroupBy = "day" | "week" | "folder" | "none";

export interface LibraryFilters {
  group?: GroupBy;
  folder?: string;
  tag?: string;
  person?: string;
  from?: string;
  to?: string;
  without_summary?: boolean;
}

/** Build a daemon URL, dropping empty filters so the query string says only what was asked. */
export function url(handshake: Handshake, path: string, params: Record<string, unknown> = {}): string {
  const query = new URLSearchParams();
  if (handshake.token) query.set("token", handshake.token);
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "" || value === false) continue;
    query.set(key, String(value));
  }
  const suffix = query.toString();
  return `http://127.0.0.1:${handshake.port}${path}${suffix ? `?${suffix}` : ""}`;
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    // The daemon answers failures with `{"error": …}`; anything else is a bug worth showing raw.
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export class LibraryClient {
  constructor(private readonly handshake: Handshake) {}

  async view(filters: LibraryFilters = {}): Promise<LibraryView> {
    return json<LibraryView>(await fetch(url(this.handshake, "/library", { ...filters })));
  }

  async search(query: string, limit = 30): Promise<SearchHit[]> {
    return json<SearchHit[]>(await fetch(url(this.handshake, "/library/search", { q: query, limit })));
  }

  async detail(id: string): Promise<MeetingDetail> {
    return json<MeetingDetail>(await fetch(url(this.handshake, `/meetings/${encodeURIComponent(id)}`)));
  }

  private async post<T>(id: string, action: string, body?: unknown): Promise<T> {
    return json<T>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(id)}/${action}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      }),
    );
  }

  moveTo(id: string, folder: string) {
    return this.post<{ folder: string }>(id, "folder", { folder });
  }

  setTags(id: string, tags: string[]) {
    return this.post<{ tags: string[] }>(id, "tags", { tags });
  }

  rename(id: string, title: string) {
    return this.post<{ title: string }>(id, "title", { title });
  }

  trash(id: string) {
    return this.post<{ trashed: boolean }>(id, "trash");
  }
}

const WEEKDAYS = ["Chủ nhật", "Thứ hai", "Thứ ba", "Thứ tư", "Thứ năm", "Thứ sáu", "Thứ bảy"];

/**
 * A heading a person reads at a glance.
 *
 * Recent days get their name — "Hôm nay", "Thứ ba" — because that is how someone thinks about a
 * meeting they remember. Older ones get the date, because by then the weekday means nothing.
 */
export function dayLabel(day: string, today: string): string {
  if (day === today) return "Hôm nay";
  if (day === shiftDay(today, -1)) return "Hôm qua";

  const date = parseDay(day);
  if (!date) return day;

  const age = daysBetween(day, today);
  if (age > 0 && age < 7) return WEEKDAYS[date.getUTCDay()] ?? day;

  const suffix = date.getUTCFullYear() === parseDay(today)?.getUTCFullYear() ? "" : ` năm ${date.getUTCFullYear()}`;
  return `${date.getUTCDate()} tháng ${date.getUTCMonth() + 1}${suffix}`;
}

/** `2026-W32` reads as a week, and a folder path reads as itself. */
export function groupLabel(key: string, group: GroupBy, today: string): string {
  if (group === "week") {
    const match = /^(\d{4})-W(\d{1,2})$/.exec(key);
    return match ? `Tuần ${Number(match[2])}, ${match[1]}` : key;
  }
  if (group === "folder") return key === "" ? "Chưa phân loại" : key;
  if (group === "day") return dayLabel(key, today);
  return key;
}

/** `2538` reads as `42 phút`, not as a number of seconds nobody converts in their head. */
export function formatDuration(seconds: number): string {
  if (seconds <= 0) return "—";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${Math.max(1, minutes)} phút`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} giờ` : `${hours} giờ ${rest} phút`;
}

/** The clock time a meeting started, taken from its own offset rather than converted to ours. */
export function timeOfDay(date: string): string {
  return date.slice(11, 16) || "";
}

/** Seconds into a recording as `12:04`, or `1:02:05` once it passes an hour. */
export function timestamp(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const s = String(total % 60).padStart(2, "0");
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Today in the local timezone as `YYYY-MM-DD`. */
export function localDay(now = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

function parseDay(day: string): Date | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(day);
  if (!match) return null;
  // UTC throughout: these are calendar days already fixed in the meeting's own offset, and letting
  // the browser reinterpret them in its timezone would move some of them a day.
  return new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
}

function shiftDay(day: string, by: number): string {
  const date = parseDay(day);
  if (!date) return day;
  date.setUTCDate(date.getUTCDate() + by);
  return date.toISOString().slice(0, 10);
}

function daysBetween(from: string, to: string): number {
  const a = parseDay(from);
  const b = parseDay(to);
  if (!a || !b) return Number.NaN;
  return Math.round((b.getTime() - a.getTime()) / 86_400_000);
}
