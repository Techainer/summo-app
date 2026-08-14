/**
 * The wire format the daemon speaks.
 *
 * Mirrors `crates/summo-engine/src/protocol.rs` and `crates/summo-core/src/event.rs`. Keeping the
 * two in step by hand is a known cost; the alternative is a codegen step that would have to run on
 * every developer's machine before the UI compiles.
 */

export type Lane = "mic" | "system";

export type SegmentSource = "partial" | "final" | "revised" | "manual";

export interface Word {
  text: string;
  t0: number;
  t1: number;
  conf?: number;
}

export interface Segment {
  seq: number;
  lane: Lane;
  text: string;
  t0: number;
  t1: number;
  source: SegmentSource;
  speaker?: string;
  conf?: number;
  words?: Word[];
  /**
   * A live translation of this line, when one was asked for.
   *
   * Held beside the text rather than replacing it: the original is the record of what was said, and
   * anyone checking a subtitle against the speaker needs both on screen.
   */
  translation?: { lang: string; text: string };
}

export type Event =
  | ({ kind: "partial" } & Segment)
  | ({ kind: "final" } & Segment)
  | ({ kind: "revise" } & Segment)
  | { kind: "translation"; seq: number; lang: string; text: string }
  | { kind: "speaker_rename"; from: string; to: string }
  | { kind: "progress"; id: string; pct: number; stage: string; eta_s?: number }
  | { kind: "stat"; rtf: number; rss_mb: number; queue_ms: number }
  | { kind: "info"; text: string }
  | { kind: "error"; message: string; transient: boolean };

export interface SessionSpec {
  /** Empty means "whatever the settings say"; the daemon resolves it. */
  live_model: string;
  refine_model?: string;
  lanes?: Lane[];
  language?: string;
  diarize?: boolean;
  /** Translate finished lines into this language as they land. */
  translate_to?: string;
  device_id?: string;
}

export type Command =
  | ({ cmd: "session_start" } & SessionSpec)
  | { cmd: "session_stop" }
  | { cmd: "model_load"; id: string }
  | { cmd: "model_pull"; id: string }
  // Both fields optional on purpose: the interface changes a language and lets the daemon pick the
  // model that hears it, while a client comparing two models names one and keeps the language.
  | { cmd: "model_swap"; id?: string; language?: string }
  | { cmd: "ping" };

/** Whether an event carries transcript text. */
export function isTranscript(
  event: Event,
): event is { kind: "partial" | "final" | "revise" } & Segment {
  return event.kind === "partial" || event.kind === "final" || event.kind === "revise";
}

/**
 * Whether an incoming update may replace what is already displayed.
 *
 * Mirrors `SegmentSource::accepts` in Rust, and exists for the same reason: a partial re-decode can
 * arrive after the final it belongs to, and a hand edit must never be overwritten by a model.
 */
export function accepts(current: SegmentSource, next: SegmentSource): boolean {
  if (current === "manual") return false;
  if (next === "manual") return true;
  if (current === "partial") return true;
  if (current === "final") return next === "final" || next === "revised";
  return next === "revised";
}

/** Encode a PCM frame: one lane tag byte, then little-endian f32 samples. */
export function encodeFrame(lane: Lane, samples: Float32Array): ArrayBuffer {
  const buffer = new ArrayBuffer(1 + samples.length * 4);
  const view = new DataView(buffer);
  view.setUint8(0, lane === "mic" ? 0 : 1);
  for (let i = 0; i < samples.length; i += 1) {
    view.setFloat32(1 + i * 4, samples[i] ?? 0, true);
  }
  return buffer;
}

/** Seconds to `MM:SS`, or `H:MM:SS` past an hour. */
export function formatTime(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${pad(m)}:${pad(s)}`;
}
