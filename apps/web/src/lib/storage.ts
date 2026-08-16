import { url } from "./library";
import type { Handshake } from "./engine";

/**
 * What Summo is using on disk, and getting it back.
 *
 * The daemon has reported this since the vault was written and enforced a retention policy on every
 * start, and no screen had ever asked it. Audio is the largest thing this app writes — a transcript
 * is kilobytes, its recording megabytes — and the one a person is most likely to want gone.
 */

export interface Recording {
  id: string;
  title: string;
  /** `YYYY-MM-DD`, empty when the meeting is gone and only its audio is left. */
  day: string;
  bytes: number;
  files: number;
  path: string;
}

export interface Usage {
  vault_bytes: number;
  audio_bytes: number;
  /** Model blobs, shared between meetings — not the cost of any one recording. */
  model_bytes: number;
  total_bytes: number;
  /** Largest first, as the daemon sorted them. */
  recordings: Recording[];
  /** Audio with no meeting left to explain it. */
  orphaned: Recording[];
}

export interface Pruned {
  removed: Recording[];
  attachments?: string[];
  freed_bytes: number;
  /** True when nothing was actually deleted. */
  dry_run: boolean;
}

export interface StoragePolicy {
  /** Days to keep audio; zero keeps it forever. */
  keep_days: number;
  keep_audio: boolean;
}

async function body<T>(response: Response): Promise<T> {
  const parsed: unknown = await response.json();
  if (!response.ok) {
    const error = (parsed as { error?: unknown }).error;
    throw new Error(typeof error === "string" ? error : response.statusText);
  }
  return parsed as T;
}

export class StorageClient {
  constructor(private readonly handshake: Handshake) {}

  usage(): Promise<Usage> {
    return fetch(url(this.handshake, "/storage")).then((r) => body<Usage>(r));
  }

  /**
   * Delete recordings past the retention setting.
   *
   * `dry` is the default at both ends. The daemon treats a request with no parameter as a dry run
   * because a mistyped URL is not consent, and this screen shows what *would* go before it offers
   * to do it — the one thing here that cannot be undone.
   */
  prune(dry = true): Promise<Pruned> {
    return fetch(url(this.handshake, "/storage/prune", { dry_run: dry ? "true" : "false" }), {
      method: "POST",
    }).then((r) => body<Pruned>(r));
  }

  /** Save one field, or both. Absent fields are left as they are. */
  policy(next: Partial<StoragePolicy>): Promise<StoragePolicy> {
    return fetch(url(this.handshake, "/settings/storage"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(next),
    }).then((r) => body<StoragePolicy>(r));
  }
}

/**
 * Bytes, as a person would say them.
 *
 * Binary units against decimal prefixes on purpose: this is a number next to a disk, and every file
 * manager the user has ever looked at rounds the same way. Vietnamese uses a comma for the decimal
 * mark, so the number is formatted by the locale rather than by `toFixed`.
 */
export function bytes(count: number, locale: string): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = count;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const rounded = new Intl.NumberFormat(locale, {
    maximumFractionDigits: unit === 0 ? 0 : value < 10 ? 1 : 0,
  }).format(value);
  return `${rounded} ${units[unit]}`;
}
