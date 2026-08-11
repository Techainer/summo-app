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

/** Hours and minutes, as a person would say them. */
export function duration(seconds: number): string {
  if (seconds <= 0) return "—";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} phút`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours} giờ` : `${hours} giờ ${rest} phút`;
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
