import type { Lane } from "./protocol";

/**
 * What to capture, and what to do with it.
 *
 * Two settings that belong together because they are the same decision from the user's side: *what
 * am I recording, and do I want it in another language*. Turning on system audio and a target
 * language is the whole "watch a talk you do not speak the language of" feature — there is no
 * YouTube integration because there is nothing to integrate with. The loopback already hears
 * whatever is playing.
 *
 * Kept in `localStorage` rather than the daemon's settings, for the same reason the interface
 * language is: it describes this screen's habits, not the vault, and the global shortcut has to be
 * able to read it before any network call completes.
 */

const KEY = "summo.capture";

export interface Capture {
  /** Microphone, system audio, or both. */
  lanes: Lane[];
  /**
   * The language being spoken. Empty means "let the model detect it".
   *
   * Here rather than only in the daemon's settings because it is a per-meeting decision as often as
   * it is a preference — the standup is in Vietnamese and the customer call is in English — and the
   * record bar has to be able to change it without writing to the vault's settings file. The
   * daemon's `models.language` remains the default this starts from.
   */
  spoken: string;
  /**
   * Languages to translate finished lines into as they land. Empty means off.
   *
   * Off by default and deliberately so: every line becomes a request to a language model, which
   * costs money on a hosted provider and battery on a local one. Nobody should discover that by
   * accident.
   *
   * A list, because a call can have more than one reader — and because the second target costs
   * another pass through a model that is already loaded rather than another model.
   */
  translateInto: string[];
}

export const DEFAULT: Capture = { lanes: ["mic"], translateInto: [], spoken: "" };

/**
 * Read the saved choice.
 *
 * Anything unrecognised falls back to the default rather than throwing. This is parsed from storage
 * a user or an older version wrote, and a bad value must not stop the app from recording.
 */
export function load(): Capture {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw) as Partial<Capture>;
    return normalize(parsed);
  } catch {
    return DEFAULT;
  }
}

export function save(capture: Capture): void {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(normalize(capture)));
  } catch {
    // Private browsing, or a webview with storage locked down. The choice still applies to this
    // session; it just will not be remembered.
  }
}

/**
 * Coerce whatever was stored into something the daemon will accept.
 *
 * A session with no lanes is rejected by the daemon, which would turn a corrupt preference into a
 * record button that fails — so an empty list becomes the microphone.
 */
export function normalize(input: Partial<Capture> | null | undefined): Capture {
  const lanes = (Array.isArray(input?.lanes) ? input.lanes : []).filter(
    (lane): lane is Lane => lane === "mic" || lane === "system",
  );
  const unique = [...new Set(lanes)];
  return {
    lanes: unique.length > 0 ? unique : DEFAULT.lanes,
    translateInto: targets(input),
    // Lower-cased, because a language code is compared against the manifests' own spelling and
    // `VI` from an older build must not read as a language nothing covers.
    spoken: typeof input?.spoken === "string" ? input.spoken.trim().toLowerCase() : "",
  };
}

/**
 * The saved targets, accepting the single string this used to be.
 *
 * `translateTo` was one language and is in every existing browser's local storage. Dropping it
 * would silently turn translation off for everybody who had it on, at the start of their next
 * meeting, with nothing on screen to say why.
 */
function targets(
  input: (Partial<Capture> & { translateTo?: unknown }) | null | undefined,
): string[] {
  const raw = Array.isArray(input?.translateInto)
    ? input.translateInto
    : typeof input?.translateTo === "string"
      ? [input.translateTo]
      : [];
  const clean = raw
    .filter((code): code is string => typeof code === "string")
    .map((code) => code.trim())
    .filter((code) => code.length > 0);
  return [...new Set(clean)];
}

/** Whether live translation is on. */
export function translating(capture: Capture): boolean {
  return capture.translateInto.length > 0;
}

/**
 * Whether this capture will hear anything other than the person holding the laptop.
 *
 * Used to explain why live translation looks like it is doing nothing: translating the microphone
 * lane translates *you*, which is rarely what anyone wants and is exactly what happens if the
 * system-audio switch is forgotten.
 */
export function hearsOthers(capture: Capture): boolean {
  return capture.lanes.includes("system");
}

/**
 * Languages worth offering for live translation, each in its own name.
 *
 * Endonyms, not translations, and the one place in the app where words are written into the source
 * on purpose: a picker that renders "Vietnamese" to somebody who reads Vietnamese is asking them to
 * find their language in a language they are trying to leave. `i18n-exempt` says so to the test
 * that otherwise forbids this.
 */
export const TARGETS: { code: string; label: string }[] = [
  { code: "", label: "—" },
  { code: "vi", label: "Tiếng Việt" }, // i18n-exempt: endonym
  { code: "en", label: "English" },
  { code: "ja", label: "日本語" },
  { code: "ko", label: "한국어" },
  { code: "zh", label: "中文" },
  { code: "fr", label: "Français" },
  { code: "es", label: "Español" },
];
