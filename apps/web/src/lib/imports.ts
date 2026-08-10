import type { Handshake } from "./engine";
import { url } from "./library";

/**
 * Importing a recording that was made somewhere else.
 *
 * The whole point of this module is that an import is *slow* — minutes for a long meeting — and the
 * user should be able to walk away. So starting one returns a job, and the screen polls it. The
 * daemon holds the job list, not this client, which is why a reload does not lose an import that is
 * still running.
 *
 * The browser cannot hand the daemon a file: it has a `File`, and the daemon wants a path it can
 * give to ffmpeg. In Tauri the file dialog returns a real path, which is what {@link pickFile}
 * uses. In a plain browser there is no such thing, so the UI asks the user to type or paste a path
 * instead of pretending a drag-and-drop will work.
 */

export type State = "queued" | "extracting" | "running" | "done" | "failed";

export interface Job {
  id: string;
  title: string;
  source: string;
  state: State;
  /** Present while running. */
  done_s?: number;
  total_s?: number;
  segments?: number;
  /** Present once done. */
  meeting?: string;
  path?: string;
  duration_s?: number;
  /** Present once failed. */
  error?: string;
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export interface StartOptions {
  model?: string;
  language?: string;
  diarize?: boolean;
}

export class ImportClient {
  constructor(private readonly handshake: Handshake) {}

  async start(path: string, options: StartOptions = {}): Promise<Job> {
    return json<Job>(
      await fetch(url(this.handshake, "/imports"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ path, ...options }),
      }),
    );
  }

  async list(): Promise<Job[]> {
    return json<Job[]>(await fetch(url(this.handshake, "/imports")));
  }

  async get(id: string): Promise<Job> {
    return json<Job>(await fetch(url(this.handshake, `/imports/${encodeURIComponent(id)}`)));
  }

  async clearFinished(): Promise<number> {
    const body = await json<{ cleared: number }>(
      await fetch(url(this.handshake, "/imports/clear"), { method: "POST" }),
    );
    return body.cleared;
  }
}

/** Whether a job has stopped moving. */
export function isFinished(job: Job): boolean {
  return job.state === "done" || job.state === "failed";
}

/**
 * How often to ask.
 *
 * Two seconds, not the fifteen minutes the nudge poll uses: this one only runs while a progress bar
 * is on screen and the user is watching it, and a bar that jumps in ten-second steps reads as
 * frozen.
 */
export const POLL_MS = 2_000;

/**
 * A percentage for the bar, or `null` when there is nothing honest to show.
 *
 * Returning 0 for "length unknown" would render an empty bar that looks stuck, which is the exact
 * impression a long import must not give.
 */
export function percent(job: Job): number | null {
  if (job.state === "done") return 100;
  if (job.state !== "running") return null;
  const total = job.total_s ?? 0;
  if (total <= 0) return null;
  const fraction = (job.done_s ?? 0) / total;
  return Math.round(Math.min(1, Math.max(0, fraction)) * 100);
}

/** One line describing where a job is, in the user's language. */
export function describe(job: Job): string {
  switch (job.state) {
    case "queued":
      return "Đang chờ";
    case "extracting":
      return "Đang tách âm thanh";
    case "running": {
      const pct = percent(job);
      const found = job.segments ?? 0;
      const head = pct === null ? "Đang nhận dạng" : `Đang nhận dạng — ${pct}%`;
      return found > 0 ? `${head}, ${found} câu` : head;
    }
    case "done":
      return `Xong — ${job.segments ?? 0} câu`;
    case "failed":
      return job.error ?? "Lỗi";
  }
}

/**
 * The file's own name, for when the job has none yet.
 *
 * Handles both separators, because a Windows path arriving over the socket is normal and splitting
 * on `/` alone would leave the whole path as the "name".
 */
export function baseName(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** Extensions the file dialog should offer. Mirrors `summo_media::SUGGESTED`. */
export const SUGGESTED = [
  "mp3",
  "m4a",
  "wav",
  "flac",
  "ogg",
  "opus",
  "mp4",
  "mkv",
  "webm",
  "mov",
] as const;

/**
 * Ask the OS for a file, when running inside Tauri.
 *
 * Resolves to `null` outside Tauri or when the user cancels — both are "no file", and the caller
 * should not have to tell them apart. The import is loaded lazily so a browser build never pulls in
 * the plugin.
 */
export async function pickFile(): Promise<string | null> {
  const tauri = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  if (!tauri) return null;
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const chosen = await open({
      multiple: false,
      filters: [{ name: "Ghi âm & video", extensions: [...SUGGESTED] }],
    });
    return typeof chosen === "string" ? chosen : null;
  } catch {
    // A missing plugin should degrade to the typed-path field, not break the screen.
    return null;
  }
}
