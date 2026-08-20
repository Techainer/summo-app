import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Reading the day/week report the daemon computes.
 *
 * No model runs behind this — `summo-vault`'s `report.rs` adds up what the vault already holds — so
 * it is instant and works offline. That is worth knowing at the call site: this can be fetched on
 * every navigation without a cost or a spinner.
 */

export interface ActionItem {
  text: string;
  owner?: string;
  meeting: string;
  meeting_title: string;
  day: string;
  done: boolean;
}

export interface PersonTime {
  name: string;
  meetings: number;
  seconds: number;
}

export interface ReportMeeting {
  id: string;
  title: string;
  day: string;
  duration: number;
  participants: string[];
  tags: string[];
  has_summary: boolean;
}

export interface Report {
  from: string;
  to: string;
  meetings: ReportMeeting[];
  total_seconds: number;
  people: PersonTime[];
  tags: [string, number][];
  open_actions: ActionItem[];
  done_actions: number;
  without_summary: string[];
  quiet_days: string[];
}

export class ReportClient {
  constructor(private readonly handshake: Handshake) {}

  /** Omit both bounds for today; omit `from` for a single day. */
  async between(from?: string, to?: string): Promise<Report> {
    return readJson<Report>(await fetch(url(this.handshake, "/report", { from, to })));
  }
}

/** `YYYY-MM-DD` shifted by whole days, in UTC so a timezone cannot move the boundary. */
export function shiftDay(day: string, days: number): string {
  const date = new Date(`${day}T00:00:00Z`);
  if (Number.isNaN(date.getTime())) return day;
  date.setUTCDate(date.getUTCDate() + days);
  return date.toISOString().slice(0, 10);
}

/** Today, in the machine's own timezone — a report for "today" means the user's today. */
export function today(): string {
  const now = new Date();
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 10);
}

/**
 * Every day in the report's window, with what was recorded on it.
 *
 * The report carries meetings and a list of quiet days, and the screen drew the second of those as
 * a comma-separated list of dates — "Không họp: 2026-08-14, 2026-08-15, …" — under a screen that
 * was otherwise empty. The same facts as a row of bars answer the question somebody opened this
 * screen with, which is when the work happened, and a quiet day is then a gap rather than a
 * sentence.
 *
 * Built from the window rather than from the meetings, so days with nothing on them are rows with a
 * zero in them and the shape of a week stays honest. Capped: a range wider than `limit` days is not
 * a strip a person reads, and no range the interface offers reaches it.
 */
export function byDay(
  report: Report,
  limit = 31,
): { day: string; seconds: number; count: number }[] {
  const totals = new Map<string, { seconds: number; count: number }>();
  for (const meeting of report.meetings) {
    const at = totals.get(meeting.day) ?? { seconds: 0, count: 0 };
    at.seconds += meeting.duration;
    at.count += 1;
    totals.set(meeting.day, at);
  }

  const out: { day: string; seconds: number; count: number }[] = [];
  for (let day = report.from; out.length < limit; day = shiftDay(day, 1)) {
    out.push({ day, ...(totals.get(day) ?? { seconds: 0, count: 0 }) });
    if (day === report.to) break;
    // A malformed window would otherwise walk forever: `shiftDay` returns its input unchanged when
    // it cannot parse it, so the date would never reach `to`.
    if (shiftDay(day, 1) === day) break;
  }
  return out;
}

/**
 * Share of the total, as a percentage, for a bar's width.
 *
 * Returns 0 rather than `NaN` when the total is zero, so an empty report renders flat bars instead
 * of a broken layout.
 */
export function share(value: number, total: number): number {
  return total > 0 ? Math.round((value / total) * 100) : 0;
}
