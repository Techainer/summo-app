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
  /**
   * The best *installed* model for this language — what a recording would actually use.
   *
   * `model` above is the best that exists, downloaded or not, and conflating the two is what made
   * every screen name a model the machine does not have. On a laptop with only Whisper, Vietnamese
   * reported `gipformer-65m, installed: false`, so the interface announced Gipformer would be
   * listening. Whisper was, and the transcript was fine — only the description was wrong, and it
   * read as though the app were about to swap models on its own.
   *
   * `null` only when nothing here covers the language at all.
   */
  serving: string | null;
  serving_name: string | null;
  /** Measured accuracy of `serving`, which is the number you will actually get today. */
  serving_accuracy: number;
}

/**
 * A better model exists for this language than the one that would run, and it is worth offering.
 *
 * Worth offering, not worth doing: the app must never swap a model on somebody's behalf. It returns
 * the recommendation only when there is a real gap to close — something is already serving the
 * language, something better exists, and the difference is big enough to be worth a download.
 *
 * Five points of accuracy is the floor. Below that the advice costs more attention than it saves,
 * and the numbers come from different benchmark runs anyway.
 */
export function betterFor(language: Language | undefined): Language | undefined {
  if (!language?.serving || !language.model) return undefined;
  if (language.installed || language.model === language.serving) return undefined;
  return language.accuracy - language.serving_accuracy >= 0.05 ? language : undefined;
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
 * Language codes as a list somebody can find their own language in.
 *
 * Alphabetical by name is the obvious answer and is wrong for this list. Sorted that way in a
 * Vietnamese interface, "Tiếng Việt" lands under T — eightieth of ninety-nine, past the fold of any
 * dropdown — so a Vietnamese speaker who installed Whisper concluded it does not support
 * Vietnamese. It supports ninety-nine languages and hid theirs behind seventy-nine others.
 *
 * So: the reader's own language first, then the interface languages Summo ships in, then everything
 * else alphabetically in the reader's language. The first two groups are three or four entries and
 * cover almost every real answer; the tail is where somebody browses, and there alphabetical is
 * right.
 *
 * Returns `{ code, label }` because every caller needs the name it sorted by, and computing
 * `Intl.DisplayNames` twice for ninety-nine codes is work nobody needs to repeat.
 */
export function ordered(codes: string[], locale: string): { code: string; label: string }[] {
  const mine = locale.toLowerCase().split("-")[0] ?? locale;
  // The interface locales, minus the reader's own — which is already the head of the list.
  const near = ["vi", "en", "ja", "zh"].filter((code) => code !== mine);
  const rank = (code: string) => {
    const base = code.toLowerCase().split("-")[0] ?? code;
    if (base === mine) return 0;
    const at = near.indexOf(base);
    return at === -1 ? 2 : 1;
  };
  return codes
    .map((code) => ({ code, label: languageName(code, locale) }))
    .sort((a, b) => {
      const byGroup = rank(a.code) - rank(b.code);
      if (byGroup !== 0) return byGroup;
      // Within the "near" group, the order of `near` rather than the alphabet: it is a shortlist,
      // and a shortlist that reorders itself per locale is a shortlist nobody learns.
      if (rank(a.code) === 1) {
        return near.indexOf(a.code.toLowerCase()) - near.indexOf(b.code.toLowerCase());
      }
      return a.label.localeCompare(b.label, locale);
    });
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

/**
 * Remember the spoken language in the daemon, not only in this browser.
 *
 * The record bar keeps a local copy because it has to answer before any request completes, and
 * because the language is a per-meeting choice as often as a standing one. But the standing one
 * belongs to the installation: a first run that picks Japanese must still record Japanese from a
 * different browser, from the tray, or from `summo transcribe`, none of which can read
 * `localStorage`.
 *
 * Failures are the caller's to ignore. Not being able to write a preference must not stop a
 * recording that is otherwise ready to start.
 */
export async function rememberLanguage(handshake: Handshake, code: string): Promise<void> {
  const response = await fetch(url(handshake, "/settings/language"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ language: code }),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
}

/** What the daemon has loaded and ready right now. */
export interface Ready {
  model: string;
  language: string | null;
}

/**
 * Ask the daemon to build a decoder now.
 *
 * Answers when it is ready — about three and a half seconds — which is what lets the caller say
 * "sẵn sàng" rather than "asked for". Called when the app opens and after a meeting ends; the
 * daemon refills its own slot after a session too, so this is a nudge, never a requirement.
 */
export async function warmUp(handshake: Handshake): Promise<Ready | null> {
  const response = await fetch(url(handshake, "/models/warm"), { method: "POST" });
  // A daemon built without recognition has no such route, and the interface is the same interface:
  // the `-nomodels` build browses a vault and cannot record, so there is nothing to warm and
  // nothing wrong. Anything else is a real failure worth surfacing.
  if (response.status === 404 || response.status === 405) return null;
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  const body = (await response.json()) as { ready: Ready | null };
  return body.ready;
}

/**
 * What is loaded, from the status endpoint the recording banner already reads.
 *
 * Three answers, and the third is the point. `undefined` means this daemon has no warming at all —
 * a build without recognition does not carry the field — and asking it to warm would be a request
 * the browser logs as a failed one on a screen where nothing is wrong. Handling the error is not
 * enough: the console entry appears whatever the code does with the response, and the shell suite
 * treats console errors as failures because they usually are.
 */
export async function readyNow(handshake: Handshake): Promise<Ready | null | undefined> {
  const response = await fetch(url(handshake, "/status"));
  if (!response.ok) return undefined;
  const body = (await response.json()) as { ready?: Ready | null };
  return "ready" in body ? (body.ready ?? null) : undefined;
}
