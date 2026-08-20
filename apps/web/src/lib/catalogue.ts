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
  "asr" | "vad" | "denoise" | "diarize-seg" | "speaker-embed" | "embed" | "translate";

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
}

/** Which job a model is being pointed at. */
export type Role = "live" | "refine" | "vad" | "speaker" | "translator";

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
  return null;
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
  async use(role: Role, model: string): Promise<void> {
    await readJson(
      await fetch(url(this.handshake, "/settings/models"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ role, model }),
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

export function tags(model: CatalogueModel): { label: string; kind: "plain" | "warn" | "good" }[] {
  const out: { label: string; kind: "plain" | "warn" | "good" }[] = [];
  // The licence, short enough to read at a glance. "FunASR Model Open Source License Agreement
  // v1.1" is a chip wider than the card it sits in, and the full text is on the model's own page —
  // what belongs here is which licence it is, not its full legal title.
  out.push({ label: shortLicense(model.license), kind: "plain" });
  if (model.langs.length > 0 && !model.langs.includes("*")) {
    // Four is what fits on a phone before the row wraps twice.
    const shown = model.langs.slice(0, 4).join(" · ");
    out.push({
      label: model.langs.length > 4 ? `${shown} +${model.langs.length - 4}` : shown,
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
