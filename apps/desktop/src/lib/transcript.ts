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
