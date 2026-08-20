import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * First run.
 *
 * Summo needs a speech model before it can do anything, and that model is hundreds of megabytes. An
 * app that opens to an empty screen and a Record button that fails is an app that gets deleted in
 * the first minute.
 *
 * The daemon answers "what stands between this user and a working recording" as a checklist rather
 * than a wizard, and this module is the client for it. The distinction is load-bearing: a wizard is
 * a sequence somebody has to finish, and a user who quits halfway is stranded outside it. A
 * checklist recomputed from the machine picks up wherever things stand.
 */

export type StepName = "models" | "ffmpeg" | "llm";

export interface Check {
  step: StepName;
  ready: boolean;
  /** Whether recording is impossible without it. Only `models` is. */
  blocking: boolean;
  detail: string;
}

export interface Status {
  acknowledged: boolean;
  can_record: boolean;
  fresh: boolean;
  /** Setup should take over the screen. Only true for a genuinely new install. */
  should_prompt: boolean;
  /** Something is wrong the user has to be told, without being blocked. */
  needs_attention: boolean;
  checks: Check[];
  hardware: { cores: number; total_ram_mb: number; os: string; arch: string };
  /**
   * Whether this build can transcribe at all.
   *
   * A fact about the binary rather than the machine: recognition is a compile-time feature, and
   * the small `--no-models` tarball does not have it. Without this the setup screen offers a
   * catalogue of models the daemon cannot load — which is exactly what the Android app did, right
   * up to failing the recording it had just said it was ready for.
   */
  recognition: boolean;
}

export interface Recommended {
  id: string;
  name: string;
  score: number;
  /**
   * The daemon's own sentence, in English.
   *
   * Kept as the last resort rather than shown. See {@link reasonOf}: the numbers it was composed
   * from travel beside it, so the same sentence can be written in the reader's language.
   */
  reason: string;
  live_capable: boolean;
  installed: boolean;
  /** Seconds of processing per second of audio, measured on hardware like this. Null when unmeasured. */
  expected_rtf?: number | null;
  /** 0 to 1, for the language that was asked about. Zero means nothing was measured. */
  accuracy?: number;
  size_bytes?: number | null;
  license?: string | null;
  redistributable?: boolean | null;
  gated?: boolean | null;
  /**
   * Something to know before installing, when there is something.
   *
   * Today that is only "this machine may not have the memory to run it" — which used to remove the
   * model from the list entirely, so a machine that misreported its free memory was offered nothing
   * at all. Downloading and running are different acts, and only the second one needs the memory.
   */
  caution?: string | null;
}

/**
 * Why this model was ranked where it was, in the reader's language.
 *
 * The daemon composes this sentence too, and its copy is English — so a Vietnamese user choosing
 * their first model read "92% accurate, 53× faster than real time on this machine" on an otherwise
 * Vietnamese screen, at the one moment the app is asking them to trust its judgement.
 *
 * It is rebuilt here rather than translated there because the daemon does not know who is reading:
 * one engine serves the desktop window, a phone on the same network and the CLI, and the interface
 * language is a property of the screen. The three numbers behind the sentence are already on the
 * wire, so nothing new is fetched to say it in Vietnamese.
 *
 * The daemon's own words remain the fallback. An older daemon that sends the sentence and not the
 * numbers is a real pairing — the desktop shell bundles its engine, but a browser can point at
 * whatever is on the port — and English is better than a blank line.
 */
export function reasonOf(
  model: Recommended,
  t: (key: string, values?: Record<string, string | number>) => string,
): string {
  const rtf = model.expected_rtf;
  const accuracy = model.accuracy ?? 0;

  // Unmeasured speed *and* unmeasured accuracy leaves nothing to say but that. Checked first so
  // the arms below can assume a number.
  if (rtf === undefined) return model.reason;

  if (rtf !== null && model.live_capable && accuracy > 0) {
    return t("setup.reason_fast", {
      accuracy: Math.round(accuracy * 100),
      times: Math.round(1 / Math.max(rtf, Number.EPSILON)),
    });
  }
  if (rtf !== null && !model.live_capable) {
    return t("setup.reason_slow", { rtf: rtf.toFixed(2) });
  }
  if (rtf === null && accuracy > 0) {
    return t("setup.reason_unmeasured", { accuracy: Math.round(accuracy * 100) });
  }
  return t("setup.reason_unknown");
}

export type InstallState = "queued" | "downloading" | "installing" | "done" | "failed";

export interface Install {
  model: string;
  name: string;
  state: InstallState;
  done?: number;
  total?: number;
  error?: string;
}

/** Why a model was left out — the daemon computes these and nothing ever showed them. */
export interface Rejected {
  id: string;
  reason: string;
}

export interface Ranking {
  models: Recommended[];
  /** Present when the catalogue could not be read: every address tried, and what each said. */
  registry_error?: string | null;
  /** Candidates that exist and were excluded, with the reason for each. */
  rejected?: Rejected[];
}

export class OnboardingClient {
  constructor(private readonly handshake: Handshake) {}

  async status(): Promise<Status> {
    return readJson<Status>(await fetch(url(this.handshake, "/onboarding")));
  }

  async complete(): Promise<void> {
    await readJson(
      await fetch(url(this.handshake, "/onboarding/complete"), {
        method: "POST",
      }),
    );
  }

  /**
   * Rank what this machine could install for `lang`.
   *
   * `registry_error` is the field that matters when `models` is empty. The daemon ranks whatever is
   * installed when it cannot reach the catalogue, so "the network is blocked" and "nothing covers
   * Japanese" arrive as the same empty array — and the screen told a Vietnamese user, in an app
   * whose default model is Vietnamese, that no model covers their language.
   */
  async recommend(lang: string): Promise<Ranking> {
    return readJson<Ranking>(await fetch(url(this.handshake, "/onboarding/recommend", { lang })));
  }

  async install(id: string): Promise<Install> {
    return readJson<Install>(
      await fetch(url(this.handshake, "/installs"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id }),
      }),
    );
  }

  async installs(): Promise<Install[]> {
    return readJson<Install[]>(await fetch(url(this.handshake, "/installs")));
  }
}

/** How often to ask while a download is running. */
export const POLL_MS = 1_000;

export function isFinished(install: Install): boolean {
  return install.state === "done" || install.state === "failed";
}

/**
 * A percentage for a download bar, or `null` when the size is not known yet.
 *
 * Never 0 for "unknown". This is the first thing a new user watches the app do, and a bar frozen at
 * zero while a 400 MB file negotiates TLS reads as a hang.
 */
export function percent(install: Install): number | null {
  if (install.state === "done") return 100;
  if (install.state !== "downloading") return null;
  const total = install.total ?? 0;
  if (total <= 0) return null;
  return Math.round(Math.min(1, Math.max(0, (install.done ?? 0) / total)) * 100);
}

/** Bytes as something a person reads. Binary units, because that is what a download manager shows. */
export function size(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/**
 * The one thing standing in the way, if there is one.
 *
 * Only blocking steps count. Telling a user who wants to record a meeting that ffmpeg is missing is
 * true, unhelpful, and the reason setup screens have four steps when they need one.
 */
export function blocker(status: Status): Check | null {
  return status.checks.find((check) => check.blocking && !check.ready) ?? null;
}

/** Steps worth mentioning but not worth stopping for. */
export function optional(status: Status): Check[] {
  return status.checks.filter((check) => !check.blocking && !check.ready);
}

/**
 * The model to preselect.
 *
 * The daemon ranks by score; this prefers one that can keep up with live audio, because the first
 * thing a new user does is press Record, and a model that transcribes at 3× real time turns that
 * into a bad first impression however good its accuracy is.
 */
export function preferred(models: Recommended[]): Recommended | null {
  return models.find((m) => m.installed) ?? models.find((m) => m.live_capable) ?? models[0] ?? null;
}

/**
 * Whether a model needs the user to accept something before it can be fetched.
 *
 * Gated models need an upstream token; non-redistributable ones are fetched from their original
 * host under their own licence, and Summo is not the distributor. Both are legitimate choices and
 * both have to be visible before the click, not after.
 */
export function needsConsent(model: Recommended): boolean {
  return model.gated === true || model.redistributable === false;
}
