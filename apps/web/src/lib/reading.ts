/**
 * The least an utterance has to be for these rules to apply.
 *
 * Deliberately not `Segment`: there are two of those — a live one from the event stream and a
 * stored one read back out of the vault — and both views need the same reading rules. Stating the
 * requirement instead of importing one of them is what lets the recording screen and the meeting
 * screen agree about who is talking over whom, rather than each deciding for itself.
 *
 * `t1` and `lane` are optional because the stored shape does not always carry them. Their absence
 * costs the overlap rule and nothing else.
 */
export interface Utterance {
  t0: number;
  t1?: number;
  speaker?: string | null;
  lane?: string;
}

/**
 * Turning a list of utterances into something a person can read while it is still arriving.
 *
 * The transcript is not a log. A log is written to be searched afterwards; this is read *during* a
 * meeting, by somebody who is also listening, and often in a language they do not speak. Three
 * things follow from that, and each is a decision made here rather than in the component:
 *
 * **A run of lines from one speaker is one thing being said.** Repeating the name above every
 * utterance turns a paragraph into a list and doubles its height, which on a phone is the
 * difference between seeing the last thirty seconds and seeing the last ten.
 *
 * **Two people talking at once is not two people taking turns.** A single microphone produces two
 * overlapping utterances, and a plain list renders them one after the other — which reads as a
 * reply. It is not a reply, and mistaking the two changes what the meeting appears to have been.
 * {@link decorate} marks the overlap so the interface can draw it as simultaneous.
 *
 * **A gap in time is a gap in the conversation.** Ten seconds of silence between two lines is
 * information; running them together loses it.
 */

/** One utterance, with what the renderer needs to know about its neighbours. */
export interface Row<T extends Utterance = Utterance> {
  segment: T;
  /** Whether to print the speaker's name above this line. */
  showSpeaker: boolean;
  /**
   * This utterance started before the previous one finished, and a different voice is speaking.
   *
   * Not a rendering hint that can be ignored: without it the two lines read as a question and an
   * answer, and there is no way for a reader to tell the difference.
   */
  overlapping: boolean;
  /** Seconds of silence before this utterance, when it is long enough to be worth showing. */
  pause: number | null;
}

/**
 * Silence worth drawing.
 *
 * Below about four seconds a pause is a breath, a thought, or the recogniser closing an utterance
 * early — marking those would put a break between two halves of one sentence. Above it, something
 * happened: a slide changed, somebody was thinking, the room went quiet.
 */
export const PAUSE_SECONDS = 4;

/** Who is speaking, as far as grouping is concerned. */
function voice(segment: Utterance): string {
  // The lane is part of the identity, not a fallback. An unnamed voice on the microphone is the
  // person holding it; an unnamed voice on the system lane is somebody on the call. Collapsing
  // both to "unknown" would group two different people into one paragraph.
  return segment.speaker ?? `lane:${segment.lane}`;
}

/**
 * Annotate each utterance with what the one before it means for how it should be drawn.
 *
 * Pure and linear: this runs on every render of a live transcript, so it cannot afford to look
 * further back than one line — and it does not need to.
 */
export function decorate<T extends Utterance>(segments: T[]): Row<T>[] {
  return segments.map((segment, index) => {
    const previous = index > 0 ? segments[index - 1] : undefined;
    if (!previous) {
      return { segment, showSpeaker: true, overlapping: false, pause: null };
    }

    const sameVoice = voice(previous) === voice(segment);
    // Without an end time there is nothing to overlap with, and guessing one from the next line's
    // start would mark every handover as simultaneous speech.
    const ended = previous.t1 ?? segment.t0;
    // Strictly before: an utterance beginning at the exact instant the last one ended is a clean
    // handover, which is the common case and not an overlap.
    const overlapping = !sameVoice && segment.t0 < ended;
    const gap = segment.t0 - ended;

    return {
      segment,
      // A new speaker always gets a name. So does the same speaker after a long silence, because
      // by then the name has scrolled away and "who said this" is the first question again.
      showSpeaker: !sameVoice || gap >= PAUSE_SECONDS,
      overlapping,
      pause: !overlapping && gap >= PAUSE_SECONDS ? gap : null,
    };
  });
}

/**
 * Whether a translation should be set in italics.
 *
 * Italic is how the interface says "this line is the machine's, not the speaker's" — and it only
 * works for scripts that have italics. CJK has none, so a browser synthesises one by shearing the
 * glyphs, which makes dense characters measurably harder to read and looks like a rendering fault.
 * The same is true of Thai, Arabic and Hebrew.
 *
 * Those languages keep the distinction through colour and size, which they already have.
 */
export function italicise(language: string): boolean {
  const primary = language.trim().toLowerCase().split(/[-_]/)[0] ?? "";
  return ![
    "zh",
    "ja",
    "ko",
    "yue",
    "th",
    "ar",
    "fa",
    "ur",
    "he",
    "hi",
    "bn",
    "ta",
    "km",
    "lo",
    "my",
  ].includes(primary);
}
