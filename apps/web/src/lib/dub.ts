import type { CatalogueModel } from "./catalogue";
import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Speaking a meeting's translation over its own recording.
 *
 * The same shape as `imports.ts`, and for the same reason: two passes of speech synthesis over an
 * hour of transcript is minutes of work, so starting one returns a job and the screen polls it. The
 * daemon holds the list, which is what makes a dub started from `summo dub` appear on the meeting's
 * page and survive a reload.
 *
 * Why there are two passes, and why the bar says which: fitting a line into the gap it came from
 * needs to know how long the line takes to say, and the only way to know that is to say it. So
 * everything is spoken once at natural speed, planned against the real durations, and spoken again
 * at the speed the plan chose. A bar that filled, reset and filled again would look broken.
 */

export type State = "queued" | "loading" | "speaking" | "mixing" | "done" | "failed";

export interface Job {
  id: string;
  /** The meeting being dubbed, so a page showing one meeting can filter to its own jobs. */
  meeting: string;
  title: string;
  lang: string;
  state: State;
  /** Present while speaking. 1 or 2 — see the note above. */
  pass?: number;
  /**
   * Lines said so far in this pass.
   *
   * Not `done`: that is also the name of the state this ends in, and one field meaning two things
   * is a bug waiting for the moment both are true.
   */
  spoken?: number;
  total?: number;
  /** Present once done — the fields of `summo_engine::dub::Report`, flattened. */
  voice?: string;
  lines?: number;
  of?: number;
  out?: string;
  duration_s?: number;
  rate?: number;
  natural?: number;
  adjusted?: number;
  overflowing?: number;
  worst_over_s?: number;
  /** Present once failed. */
  error?: string;
}

export interface StartOptions {
  /** A registry id. Omitted means the voice chosen on the models screen, then the only one. */
  voice?: string;
  /** Gain for the original underneath, `0..=1`. Omitted takes the daemon's default. */
  under?: number;
}

export class DubClient {
  constructor(private readonly handshake: Handshake) {}

  async start(meeting: string, lang: string, options: StartOptions = {}): Promise<Job> {
    return readJson<Job>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/dub`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ lang, ...options }),
      }),
    );
  }

  async list(): Promise<Job[]> {
    return readJson<Job[]>(await fetch(url(this.handshake, "/dubs")));
  }

  async get(id: string): Promise<Job> {
    return readJson<Job>(await fetch(url(this.handshake, `/dubs/${encodeURIComponent(id)}`)));
  }

  async clearFinished(): Promise<number> {
    const body = await readJson<{ cleared: number }>(
      await fetch(url(this.handshake, "/dubs/clear"), { method: "POST" }),
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
 * The same two seconds an import poll uses, and for the same reason: it runs only while a bar is on
 * screen and somebody is watching it.
 */
export const POLL_MS = 2_000;

/**
 * A percentage for the bar, or `null` when there is nothing honest to show.
 *
 * Half the bar per pass, so it fills once across work that happens twice. Mirrors
 * `summo_engine::dub::JobState::fraction`, which has the same arithmetic and its own test — this is
 * the copy the bar reads, and the two are checked against the same numbers.
 */
export function percent(job: Job): number | null {
  if (job.state === "done") return 100;
  if (job.state === "mixing") return 98;
  if (job.state !== "speaking") return null;
  const total = job.total ?? 0;
  if (total <= 0) return null;
  const within = (job.spoken ?? 0) / total;
  const offset = (job.pass ?? 1) >= 2 ? 0.5 : 0;
  return Math.round(Math.min(1, Math.max(0, offset + within / 2)) * 100);
}

/**
 * Where a job is, as something a component can render.
 *
 * Either a key to translate, or literal text — the daemon's own error names the voice that was
 * missing and the ones that would have worked, which is more useful than any wording invented here.
 */
export type Described = { key: string; values: Record<string, string | number> } | { text: string };

export function describe(job: Job): Described {
  switch (job.state) {
    case "queued":
      return { key: "dub.state_queued", values: {} };
    case "loading":
      return { key: "dub.state_loading", values: {} };
    case "speaking": {
      const pct = percent(job);
      return {
        key: pct === null ? "dub.state_speaking" : "dub.state_speaking_pct",
        values: { pass: job.pass ?? 1, percent: pct ?? 0 },
      };
    }
    case "mixing":
      return { key: "dub.state_mixing", values: {} };
    case "done":
      return { key: "dub.state_done", values: { count: job.lines ?? 0, of: job.of ?? 0 } };
    case "failed":
      return job.error ? { text: job.error } : { key: "dub.state_failed", values: {} };
  }
}

/**
 * Whether a manifest's declared languages include the one asked for.
 *
 * Mirrors `summo_models::langs_cover`. `*` is a claim to every language and a regional tag asks for
 * its base — `en-US` is served by a voice that says `en`.
 */
export function covers(langs: string[], language: string): boolean {
  const base = language.toLowerCase().split("-")[0];
  return langs.some((each) => {
    const declared = each.toLowerCase();
    return declared === "*" || declared === language.toLowerCase() || declared === base;
  });
}

/**
 * The installed voices that could speak `language`.
 *
 * Asked before the button is offered. A voice that cannot say the line is not a slower answer, it
 * is a wrong one: a VITS model handed a language it has no phoneme table for does not fail — it
 * runs the text through the table it has and says whatever comes out, confidently, over somebody's
 * meeting.
 */
export function voicesFor(installed: CatalogueModel[], language: string): CatalogueModel[] {
  return installed.filter((model) => model.task === "tts" && covers(model.langs, language));
}
