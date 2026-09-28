/**
 * The transcript, as the UI holds it.
 *
 * Segments arrive out of order and more than once: a partial, then a final, then possibly a
 * revision from a slower model minutes later. The store keeps one entry per sequence number and
 * applies the same precedence rules the daemon uses, so text never flickers backwards.
 */

import { accepts, isTranscript, type Event, type Segment } from "./protocol";

export interface TranscriptState {
  segments: Segment[];
  /** Index into `segments` by sequence number, so an update is O(1) rather than a scan. */
  index: Map<number, number>;
}

export function empty(): TranscriptState {
  return { segments: [], index: new Map() };
}

/**
 * Apply one event. Returns the same object when nothing changed, so React can skip a re-render.
 */
export function apply(state: TranscriptState, event: Event): TranscriptState {
  // A translation arrives seconds after the line it belongs to — a model round trip, not a decode.
  // It attaches to the segment rather than replacing it: the original is what was actually said,
  // and a viewer checking a subtitle against the speaker needs both.
  if (event.kind === "translation") return translate(state, event.seq, event.lang, event.text);

  if (!isTranscript(event)) return state;

  const { kind, ...segment } = event;
  const incoming: Segment = {
    ...segment,
    source: kind === "partial" ? "partial" : kind === "final" ? "final" : "revised",
  };

  const existing = state.index.get(incoming.seq);
  if (existing === undefined) {
    const segments = [...state.segments, incoming];
    const index = new Map(state.index).set(incoming.seq, segments.length - 1);
    return { segments, index };
  }

  const current = state.segments[existing];
  if (!current || !accepts(current.source, incoming.source)) return state;

  const segments = state.segments.slice();
  const language = incoming.language ?? current.language;
  segments[existing] = {
    ...current,
    ...incoming,
    // A revision without a speaker must not erase one diarization already assigned.
    speaker: incoming.speaker ?? current.speaker,
    // Nor a subtitle into a language the line has turned out not to need.
    //
    // Live translation runs on the line as first heard. A second model may correct which language
    // that was several hundred milliseconds later, and by then the subtitle has been requested,
    // paid for and drawn. So a Vietnamese sentence the fast model called English arrives with a
    // Vietnamese "translation" underneath it — reported as "đang ở vi sao còn dịch vi nữa".
    //
    // The daemon already refuses these on the way out; `translate::same_language` is the rule.
    // This is the same rule applied at the only other moment it can be: once the language is
    // known. Dropping is right rather than hiding — the line is the translation.
    translations: sameLanguage(language, current.translations),
  };
  return { segments, index: state.index };
}

/**
 * Subtitles that are not in the language the line was spoken in.
 *
 * Region is a spelling of a language, not a different one: a line heard as `en-US` does not need
 * an `en` subtitle. The daemon compares them the same way — see `translate::same_language`.
 */
function sameLanguage(
  spoken: string | undefined,
  translations: Segment["translations"],
): Segment["translations"] {
  if (!spoken || !translations?.length) return translations;
  const base = (code: string) => code.toLowerCase().split(/[-_]/)[0] ?? "";
  const heard = base(spoken);
  const kept = translations.filter((each) => base(each.lang) !== heard);
  return kept.length === translations.length ? translations : kept;
}

/**
 * Attach a translation to a segment.
 *
 * A translation for a `seq` that has not arrived is dropped rather than held: out-of-order delivery
 * would mean inventing a segment with no text, no speaker and no timing, which then renders as a
 * blank line in the transcript.
 *
 * Added to the line's list, not put in place of what is there. The daemon sends one of these per
 * target language and this used to keep the last one, so asking for two subtitles produced one —
 * and which one depended on the order two network requests happened to come home in. A second
 * translation *into the same language* does replace the first: that is a line being retranslated,
 * not a second reader.
 */
export function translate(
  state: TranscriptState,
  seq: number,
  lang: string,
  text: string,
): TranscriptState {
  const at = state.index.get(seq);
  if (at === undefined) return state;
  const current = state.segments[at];
  if (!current) return state;

  const existing = current.translations ?? [];
  const already = existing.findIndex((each) => each.lang === lang);
  const translations =
    already === -1
      ? [...existing, { lang, text }]
      : existing.map((each, i) => (i === already ? { lang, text } : each));

  const segments = state.segments.slice();
  segments[at] = { ...current, translations };
  return { segments, index: state.index };
}

/** Mark a segment as hand-edited, which freezes it against further model output. */
export function edit(state: TranscriptState, seq: number, text: string): TranscriptState {
  const at = state.index.get(seq);
  if (at === undefined) return state;
  const current = state.segments[at];
  if (!current) return state;

  const segments = state.segments.slice();
  segments[at] = { ...current, text, source: "manual" };
  return { segments, index: state.index };
}

/** Rename a speaker everywhere they appear. */
export function renameSpeaker(state: TranscriptState, from: string, to: string): TranscriptState {
  let changed = false;
  const segments = state.segments.map((s) => {
    if (s.speaker !== from) return s;
    changed = true;
    return { ...s, speaker: to };
  });
  return changed ? { segments, index: state.index } : state;
}
