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
  segments[existing] = {
    ...current,
    ...incoming,
    // A revision without a speaker must not erase one diarization already assigned.
    speaker: incoming.speaker ?? current.speaker,
  };
  return { segments, index: state.index };
}

/**
 * Attach a translation to a segment.
 *
 * A translation for a `seq` that has not arrived is dropped rather than held: out-of-order delivery
 * would mean inventing a segment with no text, no speaker and no timing, which then renders as a
 * blank line in the transcript.
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

  const segments = state.segments.slice();
  segments[at] = { ...current, translation: { lang, text } };
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
