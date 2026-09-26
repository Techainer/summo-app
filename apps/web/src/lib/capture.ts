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
   * The languages being spoken. Empty means "let the model detect it".
   *
   * A **list**, because "what is this meeting in" has no direction — that is the translation
   * question next door, which really is `from → to`. A standup with a customer on the call is
   * Vietnamese *and* English at the same time, and there was no way to say so: the app asked for
   * one spoken language, and the only way to arrange two models was to assign them roles on the
   * models screen. That asks the user to answer with the thing they do not know in order to
   * describe the thing they do.
   *
   * One entry behaves exactly as the single value it replaces: decode as this. Two or more means
   * detect per utterance and keep a specialist ready — see `SessionSpec::languages`.
   *
   * Order matters. The first is the language the meeting is mostly in, and it decides which
   * specialist is paired.
   *
   * Here rather than only in the daemon's settings because it is a per-meeting decision as often as
   * it is a preference, and the record bar has to be able to change it without writing to the
   * vault's settings file. The daemon's `models.language` remains the default this starts from.
   */
  spoken: string[];
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
  /**
   * The microphone to open. Empty means whatever the operating system calls default.
   *
   * Here for the reason `spoken` is: the recording opens the device, and it has to know which one
   * before any network call completes. The daemon's `recording.device_id` is the default this
   * starts from and the copy the settings file shows — and it was the *only* copy for several
   * releases, saved and read by nobody, so somebody with a headset and a built-in microphone could
   * name the one they wanted and be recorded by the other.
   */
  device: string;
  /**
   * Speak the translation into this language, for somebody wearing headphones. Empty is off.
   *
   * One language, and not a shorter `translateInto`. Two subtitles can share a screen; two voices
   * over each other is nobody's dub. It only means anything for a language `translateInto` already
   * covers, because a dub speaks translations.
   */
  listenIn: string;
  /** How loud, `0..=1`. */
  listenVolume: number;
}

export const DEFAULT: Capture = {
  lanes: ["mic"],
  translateInto: [],
  spoken: [],
  device: "",
  listenIn: "",
  listenVolume: 1,
};

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
  const into = targets(input);
  // Asking to hear a language nothing is translating into is a dead state: the daemon loads a voice
  // and it says nothing, for an hour, with a control showing it is on. Dropping the target drops
  // this with it — which is what somebody turning translation off means, and the alternative is a
  // dub of a language that is no longer being produced.
  const listen = typeof input?.listenIn === "string" ? input.listenIn.trim().toLowerCase() : "";
  return {
    lanes: unique.length > 0 ? unique : DEFAULT.lanes,
    translateInto: into,
    // Lower-cased, because a language code is compared against the manifests' own spelling and
    // `VI` from an older build must not read as a language nothing covers.
    //
    // A bare string is what every browser that has ever run this has in storage, and dropping it
    // would silently reset the spoken language for all of them — at the start of their next
    // meeting, with nothing on screen to say why. The same reason `translateTo` is still read
    // below.
    spoken: spokenList(input),
    // Not lower-cased: a `deviceId` is an opaque token the browser minted, and changing its case
    // changes which device it names — or names none at all.
    device: typeof input?.device === "string" ? input.device.trim() : "",
    listenIn: into.some((code) => code.toLowerCase() === listen) ? listen : "",
    // Clamped rather than trusted. A volume above one is distortion and a volume below zero
    // inverts the waveform, and both come from a storage value nobody validated on the way in.
    listenVolume:
      typeof input?.listenVolume === "number" && Number.isFinite(input.listenVolume)
        ? Math.min(1, Math.max(0, input.listenVolume))
        : 1,
  };
}

/**
 * The languages spoken, accepting the single string this used to be.
 *
 * Deduplicated and emptied of blanks, so "auto" cannot arrive as `[""]` — which would read as one
 * language named nothing, and a session asked to decode as nothing is a session that fails.
 */
function spokenList(input: (Partial<Capture> & { spoken?: unknown }) | null | undefined): string[] {
  const raw = Array.isArray(input?.spoken)
    ? input.spoken
    : typeof input?.spoken === "string"
      ? [input.spoken]
      : [];
  const clean = raw
    .filter((code): code is string => typeof code === "string")
    .map((code) => code.trim().toLowerCase())
    .filter((code) => code.length > 0);
  return [...new Set(clean)];
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

/**
 * Turn system audio on or off in the one place that decides it.
 *
 * There were two. The settings screen wrote `recording.capture_system_audio` into the daemon and
 * the recording read `lanes` out of `localStorage`, so the switch labelled "capture system audio"
 * in Settings changed a number nothing consulted — a control that does nothing, which is worse
 * than a control that is missing.
 *
 * The lanes are still local, for the reason at the top of this file: they are a property of the
 * machine in front of you rather than of the vault. What changed is that both controls now move the
 * same one, and the daemon's copy is kept in step so the settings screen shows the truth.
 */
export function setSystemAudio(capture: Capture, on: boolean): Capture {
  const lanes: Lane[] = on
    ? [...new Set<Lane>([...capture.lanes, "system"])]
    : capture.lanes.filter((lane) => lane !== "system");
  // The daemon refuses a session with no lanes, so turning system audio off on a system-only
  // capture leaves the microphone rather than nothing.
  return { ...capture, lanes: lanes.length > 0 ? lanes : ["mic"] };
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
