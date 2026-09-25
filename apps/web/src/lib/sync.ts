import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Keeping this vault in step with a folder — a NAS mount, a synced drive, a USB stick.
 *
 * A folder rather than an account, because there is no relay and nobody's server involved. Two
 * machines pointed at the same folder stay in step; what the folder holds is sealed, so whoever
 * hosts it learns sizes and timings and nothing else.
 *
 * ## The passphrase is never stored
 *
 * Not in `settings.json`, not in `localStorage`, not in this module. It is typed, sent once, used
 * for that run, and gone — which is why the field is emptied after every run and why there is no
 * "remember me". It is the only thing between somebody holding the folder and every meeting in the
 * vault, and a product that writes it down beside the folder has given away both halves.
 */

export interface SyncState {
  /** The folder, or `""` when sync has never been set up. */
  folder: string;
  /** What this machine calls itself in a conflict copy's name. */
  machine: string;
  /**
   * Why the folder cannot be used right now, or `null`.
   *
   * Chosen and unreachable is the state this is in most often — an unmounted NAS, an unplugged
   * stick — and it is not the same as "not set up". A screen with only those two could not tell
   * somebody to plug the drive in.
   */
  problem: string | null;
  /**
   * Whether this vault has synced through a folder before.
   *
   * The first run of an existing vault uploads everything. Saying that before the button is
   * pressed is the difference between a decision and a surprise.
   */
  synced_before: boolean;
}

/** What one file is about to have done to it, or had done. */
export type Action =
  "upload" | "download" | "merge" | "delete_remote" | "delete_local" | "resurrect";

export interface Step {
  path: string;
  action: Action;
  /** Present on `resurrect`: which side the surviving edit was made on. */
  edited_on?: "local" | "remote";
}

export interface Summary {
  uploaded: number;
  downloaded: number;
  merged: number;
  deleted: number;
  resurrected: number;
}

export interface Report {
  /** `false` for a dry run — nothing was written. */
  applied: boolean;
  folder: string;
  machine: string;
  summary: Summary;
  /** Every step. Populated for a dry run; empty after a real one, which reports by summary. */
  steps: Step[];
  /** Files both sides changed. Not an error: two whole files, and a decision to make. */
  conflicts: { path: string; copy: string }[];
  /** Paths the folder offered that would have escaped the vault. Reported, never acted on. */
  refused: string[];
}

export class SyncClient {
  constructor(private readonly handshake: Handshake) {}

  async state(): Promise<SyncState> {
    return readJson<SyncState>(await fetch(url(this.handshake, "/sync")));
  }

  /** Remember the folder and this machine's name. An empty folder turns sync off. */
  async configure(folder: string, machine: string): Promise<{ folder: string; machine: string }> {
    return readJson<{ folder: string; machine: string }>(
      await fetch(url(this.handshake, "/settings/sync"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ folder, machine }),
      }),
    );
  }

  /**
   * Run one sync, or plan one.
   *
   * The passphrase goes in the body and nowhere else. Not a query string: a URL is the one part of
   * a request that gets written down — into logs, into history, into a referrer — and this daemon
   * accepts its token there precisely because a token is revocable and a passphrase is not.
   */
  async run(passphrase: string, dryRun: boolean): Promise<Report> {
    return readJson<Report>(
      await fetch(url(this.handshake, "/sync"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ passphrase, dry_run: dryRun }),
      }),
    );
  }
}

/** Whether a summary describes any work at all. */
export function isQuiet(summary: Summary): boolean {
  return total(summary) === 0;
}

export function total(summary: Summary): number {
  return (
    summary.uploaded + summary.downloaded + summary.merged + summary.deleted + summary.resurrected
  );
}

/**
 * A summary as a list of `{ key, count }`, dropping the zeroes.
 *
 * Dropping them matters: "0 uploaded, 0 downloaded, 3 merged, 0 deleted, 0 restored" makes the
 * reader find the one number that is not zero. The component renders whatever comes back, so the
 * decision about *which* numbers are worth showing is here, where it can be tested.
 */
export function parts(summary: Summary): { key: string; count: number }[] {
  return (
    [
      ["sync.uploaded", summary.uploaded],
      ["sync.downloaded", summary.downloaded],
      ["sync.merged", summary.merged],
      ["sync.deleted", summary.deleted],
      ["sync.resurrected", summary.resurrected],
    ] as const
  )
    .filter(([, count]) => count > 0)
    .map(([key, count]) => ({ key, count }));
}

/** The translation key for a step's action, so an unknown action from a newer daemon still reads. */
export function actionKey(step: Step): string {
  if (step.action === "resurrect") {
    return step.edited_on === "remote" ? "sync.action_restore_here" : "sync.action_restore_there";
  }
  const known: Record<string, string> = {
    upload: "sync.action_upload",
    download: "sync.action_download",
    merge: "sync.action_merge",
    delete_remote: "sync.action_delete_there",
    delete_local: "sync.action_delete_here",
  };
  return known[step.action] ?? "sync.action_unknown";
}
