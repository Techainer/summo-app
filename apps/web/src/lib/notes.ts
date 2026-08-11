import type { Handshake } from "./engine";
import { url } from "./library";

/**
 * Notes the user typed, and the calendar they came from.
 *
 * Two clients in one module because they answer the same question from opposite ends: *what am I
 * working on that was not a recording*. A note is a meeting nobody recorded; an agenda entry is a
 * meeting that has not happened yet.
 */

export interface NoteSummary {
  id: string;
  title: string;
  /** `YYYY-MM-DD`. */
  day: string;
  file: string;
}

export interface Note {
  title: string;
  body: string;
  frontmatter: { id: string; date: string; tags?: string[] };
  sections: { heading: string; body: string }[];
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export class NoteClient {
  constructor(private readonly handshake: Handshake) {}

  async list(): Promise<NoteSummary[]> {
    return json<NoteSummary[]>(await fetch(url(this.handshake, "/notes")));
  }

  async read(id: string): Promise<Note> {
    return json<Note>(await fetch(url(this.handshake, `/notes/${encodeURIComponent(id)}`)));
  }

  async create(title: string, body = ""): Promise<{ id: string }> {
    return json<{ id: string }>(
      await fetch(url(this.handshake, "/notes"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ title, body }),
      }),
    );
  }

  /**
   * Save a note's text, and its title when the first line changed.
   *
   * The file is never renamed — see `summo_vault::note::set_body`. The heading inside the document
   * is what a person reads, and that does follow what they typed.
   */
  async save(id: string, body: string, title?: string): Promise<void> {
    await json(
      await fetch(url(this.handshake, `/notes/${encodeURIComponent(id)}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ body, ...(title ? { title } : {}) }),
      }),
    );
  }

  async remove(id: string): Promise<boolean> {
    const body = await json<{ removed: boolean }>(
      await fetch(url(this.handshake, `/notes/${encodeURIComponent(id)}`), { method: "DELETE" }),
    );
    return body.removed;
  }
}

export interface AgendaEntry {
  uid: string;
  summary: string;
  day: string;
  start_epoch: number;
  duration_s: number | null;
  location: string | null;
  conference: string | null;
  attendees: string[];
  repeats: boolean;
  calendar: string;
}

export class AgendaClient {
  constructor(private readonly handshake: Handshake) {}

  async list(): Promise<AgendaEntry[]> {
    return json<AgendaEntry[]>(await fetch(url(this.handshake, "/agenda")));
  }

  async addCalendar(path: string, name: string): Promise<void> {
    await json(
      await fetch(url(this.handshake, "/calendars"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ path, name }),
      }),
    );
  }

  async removeCalendar(name: string): Promise<boolean> {
    const body = await json<{ removed: boolean }>(
      await fetch(url(this.handshake, `/calendars/${encodeURIComponent(name)}`), {
        method: "DELETE",
      }),
    );
    return body.removed;
  }
}

/**
 * How long after saving to wait before saving again.
 *
 * Two seconds after the last keystroke. A note is a file on disk and every save rewrites it, so
 * saving per keystroke would rewrite a file forty times a sentence — and a shorter debounce does not
 * make anything feel faster, because nobody is waiting on it.
 */
export const SAVE_DEBOUNCE_MS = 2_000;

/**
 * Split a note's body into a title suggestion and the rest.
 *
 * A person types the title as the first line and expects it to become the title. Asking for one in
 * a separate field first is the step that makes people close a note-taking app.
 */
export function titleFrom(body: string): { title: string; rest: string } {
  const lines = body.split("\n");
  const first = (lines[0] ?? "").replace(/^#+\s*/, "").trim();
  return { title: first, rest: lines.slice(1).join("\n").trim() };
}

/** Group entries by day, newest first within each. */
export function byDay<T extends { day: string }>(entries: T[]): [string, T[]][] {
  const groups = new Map<string, T[]>();
  for (const entry of entries) {
    const bucket = groups.get(entry.day);
    if (bucket) bucket.push(entry);
    else groups.set(entry.day, [entry]);
  }
  return [...groups.entries()].sort((a, b) => b[0].localeCompare(a[0]));
}

/**
 * `HH:MM` as the calendar wrote it.
 *
 * Read with `getUTC*` deliberately, and it is worth being precise about why. The daemon turns a
 * local time with an unresolved `TZID` into an epoch *as if it were UTC* — see
 * `summo_calendar::When::approx_epoch` — so reading it back as UTC returns the wall-clock time the
 * calendar actually contains. A 09:00 standup shows as 09:00, which is what a person calls it.
 *
 * The trade is real: an event written as a true `Z` instant renders in UTC rather than in the
 * reader's zone. Business calendars overwhelmingly write `TZID`, so this is right far more often
 * than the alternative — and getting it right for both needs a timezone database the daemon
 * deliberately does not carry.
 */
export function clock(epoch: number): string {
  const at = new Date(epoch * 1000);
  return `${String(at.getUTCHours()).padStart(2, "0")}:${String(at.getUTCMinutes()).padStart(2, "0")}`;
}

/** A duration a person reads, or nothing when the calendar did not say. */
export function length(seconds: number | null): string {
  if (seconds === null || seconds <= 0) return "";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours}h` : `${hours}h${rest}`;
}

/** Which conferencing service a link points at, for a recognisable label. */
export function service(link: string | null): string | null {
  if (!link) return null;
  if (link.includes("meet.google.com")) return "Meet";
  if (link.includes("zoom.us")) return "Zoom";
  if (link.includes("teams.")) return "Teams";
  if (link.includes("whereby.com")) return "Whereby";
  if (link.includes("meet.jit.si")) return "Jitsi";
  return "Link";
}
