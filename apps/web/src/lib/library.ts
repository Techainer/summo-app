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
  /**
   * Whether this was recorded or typed.
   *
   * The library lists both — searching for a word should find the note somebody wrote about it as
   * readily as the meeting where it was said — so a reader needs to be told which is which.
   */
  kind: "meeting" | "note";
  id: string;
  title: string;
  folder: string;
  /**
   * The page this one lives inside, when it is a sub-page.
   *
   * A folder and a parent are two structures over the same set, and both are the user's: a folder
   * is where the file *is*, a parent is what the page is *part of*. Nesting a page does not move
   * its file — see `summo_vault::library::set_parent`.
   */
  parent: string | null;
  /** ISO-8601 with the offset the meeting happened in. */
  date: string;
  /** `YYYY-MM-DD`, in that same offset. */
  day: string;
  duration: number;
  participants: string[];
  tags: string[];
  /**
   * A palette name, already resolved by the daemon — `teal`, never `#4fd1d9`.
   *
   * The vault may hold either; what arrives here is only ever one of the names the theme has a
   * swatch for. See `crates/summo-vault/src/colour.rs` for why that conversion happens there.
   */
  color: string | null;
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
  /** Only colours actually in use, so the finder offers filters that would find something. */
  colours: { name: string; count: number }[];
  /** Every colour that can be set, in picker order. The daemon owns this list. */
  palette: string[];
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

/** What a vault entry is. A recording has a transcript; a note is typed. */
export type Kind = "meeting" | "note";

export interface LibraryFilters {
  group?: GroupBy;
  /** One kind, or absent for both — which is what the workspace shows by default. */
  kind?: Kind;
  /** A folder path; `""` is the vault root, meaning things nobody has filed. */
  folder?: string;
  /** Comma-separated; a document must carry every one of them. */
  tag?: string;
  colour?: string;
  person?: string;
  from?: string;
  to?: string;
  without_summary?: boolean;
}

/** Build a daemon URL, dropping empty filters so the query string says only what was asked. */
export function url(
  handshake: Handshake,
  path: string,
  params: Record<string, unknown> = {},
): string {
  const query = new URLSearchParams();
  if (handshake.token) query.set("token", handshake.token);
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === null || value === "" || value === false) continue;
    // Only what a query string can carry. An object reaching here would be serialised as
    // "[object Object]" and sent as a filter, which the daemon would then answer literally.
    if (typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean") {
      throw new TypeError(`query parameter ${key} must be a scalar, got ${typeof value}`);
    }
    query.set(key, String(value));
  }
  const suffix = query.toString();
  return `http://127.0.0.1:${handshake.port}${path}${suffix ? `?${suffix}` : ""}`;
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    // The daemon answers failures with `{"error": …}`; anything else is a bug worth showing raw.
    const body = (await response.json().catch(() => null)) as {
      error?: string;
    } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export class LibraryClient {
  constructor(private readonly handshake: Handshake) {}

  async view(filters: LibraryFilters = {}): Promise<LibraryView> {
    const { folder, ...rest } = filters;
    // `folder: ""` is the vault root — a real place to narrow to, and the one folder a query
    // parameter cannot name, since `folder=` reads as absent rather than as empty. It travels as
    // its own flag, which also means there is no sentinel path for a `..` to be smuggled into.
    const where = folder === "" ? { ...rest, unfiled: true } : { ...rest, folder };
    return json<LibraryView>(await fetch(url(this.handshake, "/library", where)));
  }

  async search(query: string, limit = 30): Promise<SearchHit[]> {
    return json<SearchHit[]>(
      await fetch(url(this.handshake, "/library/search", { q: query, limit })),
    );
  }

  async detail(id: string): Promise<MeetingDetail> {
    return json<MeetingDetail>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(id)}`)),
    );
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

  /**
   * Put a page inside another, or `null` to take it back out to the top level.
   *
   * The daemon refuses a page nested under one of its own descendants, because a loop in this tree
   * is not a wrong drawing but an infinite one.
   */
  nestUnder(id: string, parent: string | null) {
    return this.post<{ parent: string | null }>(id, "parent", { parent });
  }

  setTags(id: string, tags: string[]) {
    return this.post<{ tags: string[] }>(id, "tags", { tags });
  }

  /** `null` clears it. */
  setColour(id: string, colour: string | null) {
    return this.post<{ colour: string | null }>(id, "colour", { colour });
  }

  rename(id: string, title: string) {
    return this.post<{ title: string }>(id, "title", { title });
  }

  trash(id: string) {
    return this.post<{ trashed: boolean }>(id, "trash");
  }
}

/**
 * The colour a swatch paints, as a CSS value — or nothing.
 *
 * The one place in the app where a colour name from the vault turns into CSS, so the check that
 * makes that safe lives here and only here.
 *
 * It checks the *shape* rather than the membership: a name of nothing but lowercase letters cannot
 * close the `var(` it sits inside, whatever it says. Checking membership instead would mean a
 * second copy of the palette in TypeScript, and two lists that drift produce a colour the daemon
 * accepts and the app then refuses to draw. A name that shapes up but has no token simply resolves
 * to nothing, which is a missing dot rather than a broken screen.
 */
export function swatch(colour: string | null | undefined): string | undefined {
  if (!colour || !/^[a-z]+$/.test(colour)) return undefined;
  return `var(--color-swatch-${colour})`;
}

/**
 * The words a heading needs that a date formatter cannot supply.
 *
 * Passed in rather than imported so this module stays pure — it is the piece with the arithmetic in
 * it, and arithmetic is what the tests here are about.
 */
export interface DayWords {
  locale: string;
  today: string;
  yesterday: string;
  /** `{n}` is the week number, `{year}` the year. */
  week: string;
  unfiled: string;
}

/**
 * A heading a person reads at a glance.
 *
 * Recent days get their name — "Today", "Thứ ba" — because that is how someone thinks about a
 * meeting they remember. Older ones get the date, because by then the weekday means nothing.
 *
 * Weekday and date come from `Intl`, in the interface's own locale. They used to be a hardcoded
 * Vietnamese array, so an English interface showed "Hôm nay" above its meetings — and any language
 * a user added by dropping in a JSON file could never have reached this line at all.
 */
export function dayLabel(day: string, today: string, words: DayWords): string {
  if (day === today) return words.today;
  if (day === shiftDay(today, -1)) return words.yesterday;

  const date = parseDay(day);
  if (!date) return day;

  const age = daysBetween(day, today);
  if (age > 0 && age < 7) {
    // UTC throughout: a day key is a calendar date, not an instant, and formatting it in the local
    // zone moves it by one either side of midnight.
    return new Intl.DateTimeFormat(words.locale, {
      weekday: "long",
      timeZone: "UTC",
    }).format(date);
  }

  const sameYear = date.getUTCFullYear() === parseDay(today)?.getUTCFullYear();
  return new Intl.DateTimeFormat(words.locale, {
    day: "numeric",
    month: "long",
    year: sameYear ? undefined : "numeric",
    timeZone: "UTC",
  }).format(date);
}

/** `2026-W32` reads as a week, and a folder path reads as itself. */
export function groupLabel(key: string, group: GroupBy, today: string, words: DayWords): string {
  if (group === "week") {
    const match = /^(\d{4})-W(\d{1,2})$/.exec(key);
    return match
      ? words.week.replace("{n}", String(Number(match[2]))).replace("{year}", match[1] ?? "")
      : key;
  }
  if (group === "folder") return key === "" ? words.unfiled : key;
  if (group === "day") return dayLabel(key, today, words);
  return key;
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
