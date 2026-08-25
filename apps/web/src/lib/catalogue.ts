import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * The model catalogue: everything the registry offers, not only what is installed.
 *
 * Separate from `onboarding.ts`, which is about getting one blocking decision out of the way on
 * first run. This is the other half — browsing, comparing and changing your mind later — and it has
 * a different shape: every task rather than speech, every model rather than the recommended one,
 * and the facts a person needs before spending several hundred megabytes.
 */

/** What a model is for. Matches `summo_models::Task`. */
export type Task =
  "asr" | "vad" | "denoise" | "diarize-seg" | "speaker-embed" | "embed" | "translate" | "tts";

export interface CatalogueModel {
  id: string;
  name: string;
  task: Task;
  mode: string;
  langs: string[];
  license: string;
  attribution?: string | null;
  /** `false` when Summo may not host the files and the download goes upstream. */
  redistributable: boolean;
  /** The upstream host serves it only to an authenticated account. */
  gated: boolean;
  description?: string | null;
  size_bytes: number;
  installed: boolean;
  /** Whether this machine has the memory. */
  fits: boolean;
  min_ram_mb: number;
  /**
   * Whether this *build* contains a runtime that can load it.
   *
   * A property of the binary, not of the machine — the release ships the ONNX translation runtime
   * and not llama.cpp, so the two GGUF translators in the registry were offered by every build that
   * could never run them, at 0.8 GB and 2.4 GB a click. Absent from an older daemon, so it is
   * optional and read as `true` when missing: an unknown answer must not hide a model that works.
   */
  runnable?: boolean;
  /** Why not, in a sentence to show. `null` when it can run. */
  why_not?: string | null;
  /**
   * What the model costs and what it is worth, measured.
   *
   * The manifests have carried all of this since they were written and the catalogue dropped every
   * field of it, so a card could say "745 MB · MIT · 99 languages" and answer none of the three
   * questions somebody comparing two models is actually asking.
   *
   * All optional: a model with no published benchmarks says nothing rather than showing zeros,
   * because "0 % accurate" and "nobody measured this" look identical and are not.
   */
  speed?: Speed | null;
  /** Milliseconds from the end of speech to the committed line. */
  latency_ms?: number;
  /** Measured accuracy per language, best first. Empty when nothing was benchmarked. */
  accuracy?: LanguageAccuracy[];
  /** Peak resident memory while decoding, in MB. */
  rss_peak_mb?: number;
  /** Accelerators this model can use *and this machine has* — already intersected by the daemon. */
  accel?: string[];
}

export interface Speed {
  /** Seconds of compute per second of audio. Below 1.0 keeps up with a live meeting. */
  rtf: number;
  /**
   * Whether the figure was measured on hardware like this one.
   *
   * `false` means every published number is for another class of machine — today that is every
   * Apple Silicon Mac, since the benchmarks are all x86 — and the slowest one is being shown. The
   * interface has to say so; a speed presented as this machine's when it is not is the kind of
   * number people make decisions on.
   */
  measured_here: boolean;
}

export interface LanguageAccuracy {
  lang: string;
  /** `0..1`, from the published word error rate. */
  accuracy: number;
}

/** Which job a model is being pointed at. */
export type Role = "live" | "refine" | "vad" | "speaker" | "denoise" | "tts" | "translator";

/**
 * The role a task fills by default, or `null` when choosing is not a thing a user does.
 *
 * A voice detector and a speaker embedder are one each and picked by the machine; there is nothing
 * to choose between. Speech and translation are the two where a user genuinely decides, and they
 * are the two the catalogue offers more than one of.
 */
export function roleFor(task: Task): Role | null {
  if (task === "asr") return "live";
  if (task === "translate") return "translator";
  // Noise suppression is the one optional role, and the only one the daemon will not fall back
  // into: unset means off, because installing a denoiser to try it must not turn it on for every
  // meeting from then on. So its card needs a button, or the model is unreachable once installed —
  // which is exactly the state `Task::Denoise` spent every release in.
  if (task === "denoise") return "denoise";
  // A voice, for the same reason. It shipped installable and unchoosable: `summo dub` took a
  // hand-typed id, so the one model in the catalogue with a "Use" button worth pressing did not
  // have one, and publishing it had moved the problem rather than solved it.
  if (task === "tts") return "tts";
  return null;
}

/**
 * Whether a model can be installed here at all.
 *
 * Older daemons do not send `runnable`; missing means yes. Being wrong in that direction hides
 * nothing — the install then fails with the daemon's own reason, which is what happened before this
 * field existed.
 */
export function canRun(model: CatalogueModel): boolean {
  return model.runnable !== false;
}

/**
 * What each role points at after a change, as the daemon left it.
 *
 * The reply was being discarded and the screen patched with only the role just set, which is wrong
 * in the one case the daemon does more than it was asked: choosing a model as the live one *clears*
 * it from the second-pass slot, because a session cannot decode twice with the same model. The card
 * went on showing it in both.
 */
export type Chosen = Record<string, string | null>;

/** One model, loaded and run. Matches `summo_engine::verify::Check`. */
export interface Check {
  id: string;
  task: Task;
  /** Whether it loaded and ran. Not whether it is any good. */
  ok: boolean;
  /** What it produced, or why it could not. */
  detail: string;
  /** Load plus one inference, milliseconds. Dominated by the cold load. */
  millis: number;
}

export interface Catalogue {
  models: CatalogueModel[];
  /** Which model each role currently points at, keyed by [`Role`]. */
  chosen?: Record<string, string | null>;
  /**
   * Whether the registry answered.
   *
   * `false` means the list is only what is already on disk. Worth saying on screen: a catalogue
   * that is quietly short is indistinguishable from one that is complete.
   */
  reachable: boolean;
}

export class CatalogueClient {
  constructor(private readonly handshake: Handshake) {}

  async load(): Promise<Catalogue> {
    return readJson<Catalogue>(await fetch(url(this.handshake, "/catalogue")));
  }

  /**
   * Say which installed model fills a role.
   *
   * The role is named rather than inferred: `asr` fills two of them — the live model and the
   * slower one that re-decodes after it — and which is wanted is the user's decision.
   */
  async use(role: Role, model: string): Promise<Chosen> {
    const settings = await readJson<{ models?: Record<string, string | null> }>(
      await fetch(url(this.handshake, "/settings/models"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ role, model }),
      }),
    );
    return settings.models ?? {};
  }

  /**
   * Load one installed model and run it once.
   *
   * Never rejects on a model that fails — a failure is the answer, and it comes back as
   * `ok: false` with a reason. It rejects only when the daemon could not be asked.
   */
  async check(id: string): Promise<Check> {
    return readJson<Check>(
      await fetch(url(this.handshake, `/models/${encodeURIComponent(id)}/check`), {
        method: "POST",
      }),
    );
  }

  /** Delete an installed model. Returns the bytes reclaimed. */
  async remove(id: string): Promise<{ freed_bytes: number }> {
    return readJson<{ freed_bytes: number }>(
      await fetch(url(this.handshake, `/models/${encodeURIComponent(id)}`), {
        method: "DELETE",
      }),
    );
  }
}

/**
 * The order tasks are shown in.
 *
 * Speech first because it is the one thing recording cannot start without, then the models that
 * improve a recording, then the ones that do something with it afterwards. Anything with no entry
 * here sorts last rather than disappearing — a registry that adds a task should show it, not hide
 * it until the interface catches up.
 */
const TASK_ORDER: Task[] = [
  "asr",
  "translate",
  // A voice sits with the things that do something *with* a recording rather than to it: `summo
  // dub` reads a translated meeting back over its own audio.
  "tts",
  "vad",
  "speaker-embed",
  "diarize-seg",
  "denoise",
  "embed",
];

/** Group a catalogue into sections, in the order above, dropping empty ones. */
export function byTask(models: CatalogueModel[]): { task: Task; models: CatalogueModel[] }[] {
  const seen = new Map<Task, CatalogueModel[]>();
  for (const model of models) {
    const bucket = seen.get(model.task);
    if (bucket) bucket.push(model);
    else seen.set(model.task, [model]);
  }

  const rank = (task: Task) => {
    const at = TASK_ORDER.indexOf(task);
    return at === -1 ? TASK_ORDER.length : at;
  };

  return [...seen.entries()]
    .sort(([a], [b]) => rank(a) - rank(b))
    .map(([task, group]) => ({
      task,
      // Installed first, then largest last: the ones you have are the ones you came to look at, and
      // within the rest a reader is comparing sizes.
      models: [...group].sort((x, y) => {
        if (x.installed !== y.installed) return x.installed ? -1 : 1;
        return x.size_bytes - y.size_bytes;
      }),
    }));
}

/**
 * The short facts shown as chips under a model's name.
 *
 * Only what changes a decision. The licence is here because two of the models Summo can install are
 * not ours to redistribute and one is gated behind an account — a user finding that out at the
 * download is a user who has already committed.
 */
/**
 * The licence, as short as it can be said honestly.
 *
 * `MIT` and `Apache-2.0` are already short. The long ones are titles rather than names — "FunASR
 * Model Open Source License Agreement v1.1" — and the words that carry the meaning are at the
 * front.
 */
export function shortLicense(license: string): string {
  const trimmed = license
    .replace(/\s*Model Open Source License Agreement\s*/i, " ")
    .replace(/\s*Terms of Use\s*/i, " ")
    .replace(/\s+/g, " ")
    .trim();
  return trimmed.length > 22 ? `${trimmed.slice(0, 21)}…` : trimmed;
}

export function tags(
  model: CatalogueModel,
  /**
   * The language the reader is using, put first when this model covers it.
   *
   * A hundred-language model truncated to the first four shows `en · zh · de · es +95`, and the one
   * question somebody opens this screen with — can it hear *me* — is answered ninety-five entries
   * into a list they cannot see. The order in the manifest is by number of speakers, which is a
   * reasonable default and is not about this reader.
   *
   * Optional, because two callers render a card with no locale to hand and a slightly worse chip is
   * better than a required argument threaded through both.
   */
  locale?: string,
): { label: string; kind: "plain" | "warn" | "good" }[] {
  const out: { label: string; kind: "plain" | "warn" | "good" }[] = [];
  // The licence, short enough to read at a glance. "FunASR Model Open Source License Agreement
  // v1.1" is a chip wider than the card it sits in, and the full text is on the model's own page —
  // what belongs here is which licence it is, not its full legal title.
  out.push({ label: shortLicense(model.license), kind: "plain" });
  // No `*` case any more: the daemon expands it before this ever sees it — see the note beside
  // `"langs"` in `server.rs`. A model that still arrives with a star would show it as a language,
  // which is visibly odd rather than silently missing, and that is the right way round.
  if (model.langs.length > 0) {
    const mine = locale?.toLowerCase().split("-")[0];
    const first = mine && model.langs.some((code) => code.toLowerCase() === mine) ? [mine] : [];
    const rest = model.langs.filter((code) => code.toLowerCase() !== mine);
    // Four is what fits on a phone before the row wraps twice.
    const shown = [...first, ...rest].slice(0, 4);
    const more = model.langs.length - shown.length;
    out.push({
      label: more > 0 ? `${shown.join(" · ")} +${more}` : shown.join(" · "),
      kind: "plain",
    });
  }
  if (model.mode === "live") out.push({ label: "live", kind: "good" });
  if (model.gated) out.push({ label: "gated", kind: "warn" });
  // No `upstream` chip: the line under the card already names who the file comes from, and a
  // warning-coloured badge over a licence somebody else wrote reads as "something is wrong here",
  // which is not what "we are not the distributor" means.
  if (!model.fits) out.push({ label: "ram", kind: "warn" });
  return out;
}

/**
 * Whether a typed query describes this model.
 *
 * Over the id and the name because those are what somebody types, over the description because that
 * is where "streaming" or "tiếng Nhật" lives, and over the language codes because typing `ja` is the
 * fastest way to ask the only question this catalogue is really asked. Empty query matches
 * everything: a filter that hides the list until you type is a search box that has eaten the screen.
 */
export function matches(model: CatalogueModel, query: string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") return true;
  const haystack = [model.id, model.name, model.description ?? "", model.license, ...model.langs]
    .join(" ")
    .toLowerCase();
  // Every word, in any order. "whisper vi" should find the multilingual model whose name carries
  // one term and whose language list carries the other.
  return needle.split(/\s+/).every((word) => haystack.includes(word));
}

/** How much disk the installed models take, which is the number a user came to check. */
export function installedBytes(models: CatalogueModel[]): number {
  return models.filter((m) => m.installed).reduce((total, m) => total + m.size_bytes, 0);
}

/** Bytes as something a person reads, matching the onboarding screen's wording. */
export function size(bytes: number): string {
  if (bytes <= 0) return "";
  const mb = bytes / 1_000_000;
  return mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${Math.round(mb)} MB`;
}
