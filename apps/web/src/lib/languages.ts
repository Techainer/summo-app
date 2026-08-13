import type { Handshake } from "./engine";
import { url } from "./library";

/**
 * The language being spoken, which is a different question from the language of the interface.
 *
 * Setup used to infer one from the other: somebody reading Vietnamese is recording Vietnamese. That
 * is true often enough that being wrong is invisible — and being wrong means a 73 MB download that
 * cannot transcribe the meeting it was installed for. So it is asked.
 *
 * The list comes from the daemon rather than from a constant here, because the honest answer
 * depends on the registry, on the machine, and on what is already installed: which languages have a
 * model at all, which model it would be, how big, and — the part that matters most — how accurate
 * anyone has measured it to be on *that* language. `whisper-tiny` covers Vietnamese and scores
 * 34 %; a 73 MB transducer scores 91 %. A picker that shows only "covered" is recommending the
 * first one.
 */

/** One language the daemon can serve, with the model that would serve it. */
export interface Language {
  code: string;
  model: string | null;
  model_name: string | null;
  size_bytes: number;
  installed: boolean;
  /** Measured accuracy in `0..1`; `0` where nobody has measured this language. */
  accuracy: number;
  /** Whether that model keeps up with live audio on this machine. */
  live: boolean;
  /** Covered only through a multilingual model's `*`, never measured on it. */
  multilingual_only: boolean;
}

export interface Languages {
  /** What the next recording would use. `null` when nothing has been chosen. */
  current: string | null;
  languages: Language[];
}

/**
 * The code that means "let the model work it out".
 *
 * Empty rather than the string `auto`, because that is what the daemon and sherpa-onnx already
 * mean by it: an empty language is Whisper's own detection. Naming it `auto` here would need a
 * translation at the boundary, and boundaries are where those translations get forgotten.
 */
export const AUTO = "";

export async function fetchLanguages(handshake: Handshake): Promise<Languages> {
  const response = await fetch(url(handshake, "/languages"));
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return (await response.json()) as Languages;
}

/**
 * The language's name in the reader's own language, falling back to its code.
 *
 * `Intl.DisplayNames` rather than a table: the browser already ships names for every language in
 * every language, and a hand-written table would be ninety-nine entries per locale that nobody
 * would keep correct. Where the browser has no name — an old engine, an unusual code — the code
 * itself is shown, which is at least unambiguous.
 */
export function languageName(code: string, locale: string): string {
  if (code === AUTO) return code;
  try {
    const names = new Intl.DisplayNames([locale], { type: "language", fallback: "code" });
    const name = names.of(code);
    if (name && name !== code) return name;
  } catch {
    // `Intl.DisplayNames` is missing, or the code is not a valid language tag.
  }
  return code;
}

/**
 * Whether a language can be recorded right now, with nothing to download.
 *
 * The record button asks this. A model that is not installed is not an error — it is a download
 * with a progress bar — but it is the difference between pressing record and waiting for 73 MB.
 */
export function ready(language: Language | undefined): boolean {
  return Boolean(language?.installed && language.model);
}

/**
 * Whether automatic detection is possible: some installed model covers everything.
 *
 * Detection is Whisper's, and it costs a little accuracy and can flip mid-recording in a meeting
 * that switches languages — so it is offered where it works and never chosen for somebody.
 */
export function autoAvailable(languages: Language[]): boolean {
  return languages.some((language) => language.multilingual_only && language.installed);
}

/** Human-sized bytes, for a download somebody is deciding whether to start. */
export function megabytes(bytes: number): string {
  return `${Math.round(bytes / 1e6)} MB`;
}

/**
 * How to describe the accuracy of a model on a language.
 *
 * Three bands rather than a number, because the number is a word error rate on one benchmark and
 * reading it as a promise would be wrong in both directions. `unmeasured` is its own band and not
 * the bottom one: nobody has looked, which is different from having looked and found it poor.
 */
export function quality(language: Language): "good" | "poor" | "unmeasured" {
  if (language.accuracy <= 0) return "unmeasured";
  return language.accuracy >= 0.8 ? "good" : "poor";
}
