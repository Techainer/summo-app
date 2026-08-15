import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { decorate, italicise, type Row } from "../../lib/reading";
import { clock } from "../../lib/clock";
import { NameVoice } from "./NameVoice";
import type { Person, UnknownVoice } from "../../lib/people";

/**
 * The least this component needs to draw a line.
 *
 * Declared here rather than importing one of the two `Segment` types, because there are two: a
 * live one carrying `source` from the event stream, and a stored one read back out of the vault.
 * Both satisfy this, and stating the requirement keeps the component usable by either without
 * either having to grow a field for the other's benefit.
 */
export interface Line {
  text: string;
  /** Seconds from the start of the meeting. */
  t0: number;
  /** When it ended, where the source knows. Only used to tell two speakers apart from one. */
  t1?: number;
  /** Which capture this came from, so two unnamed voices are not merged into one. */
  lane?: string;
  /** `null` from the vault when nobody was identified, absent from a live event. */
  speaker?: string | null;
  /** Present only while recording; a partial is still being revised. */
  source?: string;
  /** A live translation of this line, shown underneath the original. */
  translation?: { lang: string; text: string };
}

/**
 * Everything needed to say who a voice belongs to, from here.
 *
 * Absent while recording — a voice log is written as the meeting is attributed, and asking "who is
 * `S2`?" of a transcript still arriving would be asking about a label that may not survive the
 * next utterance.
 */
export interface Naming {
  /** The voices in this meeting that belong to nobody, by their label. */
  unnamed: Record<string, UnknownVoice>;
  /** Everyone in the book, offered as one-click answers. */
  people: Person[];
  /** Which label is being written, so its controls can be disabled without freezing the rest. */
  busy: string | null;
  onName: (label: string, name: string) => void;
}

interface Props {
  segments: Line[];
  /** Playhead position, so the line being spoken can be marked. */
  at?: number;
  onSeek?: (seconds: number) => void;
  /** Reading mode uses Be Vietnam Pro and looser leading, for actually reading it back. */
  reading?: boolean;
  /**
   * Naming a speaker from the transcript.
   *
   * The voice book asks who `S2` is from a list, out of context. This is the moment the question is
   * actually answerable — you know who it is because you are reading what they said.
   */
  naming?: Naming;
}

/**
 * The transcript as one chip per utterance.
 *
 * Taken from the Coreto reference, and it earns its place for a reason beyond looking tidy: a
 * meeting transcript has no paragraphs, so rendering it as prose gives the eye nothing to hold on
 * to. One chip per utterance makes the turn-taking visible — you can see a monologue and a
 * back-and-forth apart at a glance, before reading a word.
 *
 * Virtualised because an eight-hour meeting is tens of thousands of utterances and the scroll has
 * to stay at 60fps.
 */
export function TranscriptChips({ segments, at, onSeek, reading = false, naming }: Props) {
  // Not compiled by the React Compiler, deliberately.
  //
  // `useVirtualizer` keeps a mutable instance and reads layout during render, which the compiler
  // cannot prove is safe to memoise, so it declines the whole component. `"use no memo"` states
  // that; the lint rule reports the skip regardless, so it is silenced here with the reason rather
  // than left to be scrolled past every day. This list is fast because it renders twenty rows out
  // of ten thousand, not because of memoisation — nothing is lost by not compiling it.
  "use no memo";

  const t = useT();
  const scroller = useRef<HTMLDivElement>(null);
  /** Which speaker label has its naming panel open. One at a time: two open panels is two answers
      to one question, and the second would scroll the first out of view anyway. */
  const [asking, setAsking] = useState<string | null>(null);

  // The same rules the recording screen uses. Two views of one transcript that disagreed about
  // who was talking over whom would be two answers to a question with one answer.
  const rows = useMemo(() => decorate(segments), [segments]);

  // eslint-disable-next-line react-hooks/incompatible-library -- see the note above the directive
  const virtualizer = useVirtualizer({
    count: segments.length,
    getScrollElement: () => scroller.current,
    // A rough average; the measurer corrects it as rows render.
    estimateSize: () => 64,
    overscan: 8,
  });

  // Which line is being spoken. A linear scan is fine — this runs once per timeupdate, at 4 Hz.
  const active = useMemo(() => {
    if (at === undefined) return -1;
    let found = -1;
    for (let i = 0; i < segments.length; i += 1) {
      const segment = segments[i];
      if (segment && segment.t0 <= at) found = i;
      else break;
    }
    return found;
  }, [segments, at]);

  if (segments.length === 0) {
    return <p className="text-fg-faint p-6 text-center">{t("meeting.no_transcript")}</p>;
  }

  return (
    <div ref={scroller} className="h-full overflow-y-auto px-1">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((row) => {
          const line = rows[row.index];
          if (!line) return null;
          const isActive = row.index === active;
          return (
            <div
              key={row.key}
              ref={virtualizer.measureElement}
              data-index={row.index}
              className="absolute inset-x-0 top-0 pb-2"
              style={{ transform: `translateY(${row.start}px)` }}
            >
              {line.pause !== null && (
                <p className="text-fg-faint text-micro px-3 pt-1 pb-2" data-testid="pause">
                  {t("transcript.pause", { seconds: Math.round(line.pause) })}
                </p>
              )}
              <Chip
                row={line}
                active={isActive}
                reading={reading}
                onSeek={onSeek}
                naming={naming}
                asking={asking}
                onAsk={setAsking}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Chip({
  row,
  active,
  reading,
  onSeek,
  naming,
  asking,
  onAsk,
}: {
  row: Row<Line>;
  active: boolean;
  reading: boolean;
  onSeek?: (seconds: number) => void;
  naming?: Naming;
  asking: string | null;
  onAsk: (label: string | null) => void;
}) {
  const { segment, overlapping, showSpeaker } = row;
  // Unconditionally, at the top. Below the early return this was a hook called only on some
  // renders, which is the one thing React's rules forbid outright — the hook order changes the
  // first time a chip becomes seekable and every hook after it in the tree reads somebody else's
  // state.
  const t = useT();
  const seekable = onSeek !== undefined;
  // A label nobody has claimed, in a transcript that can be corrected.
  const unnamed = segment.speaker ? naming?.unnamed[segment.speaker] : undefined;
  const open = unnamed !== undefined && asking === segment.speaker;

  const header = (
    <span className="flex items-baseline gap-2">
      {/* Only at the start of a run. Three chips in a row from one person is one turn, and
          repeating the name on each of them is what turns a paragraph into a list — the exact
          thing this component's chips exist to make visible. */}
      {segment.speaker &&
        showSpeaker &&
        (unnamed ? (
          <button
            type="button"
            onClick={() => onAsk(open ? null : segment.speaker!)}
            aria-expanded={open}
            aria-label={t("people.name_this", { label: segment.speaker })}
            className="text-accent hover:bg-accent-soft text-micro -mx-1 rounded px-1 font-semibold underline decoration-dotted underline-offset-2"
          >
            {segment.speaker}
          </button>
        ) : (
          <span className="text-fg-dim text-micro font-semibold">{segment.speaker}</span>
        ))}
      <span className="tabular text-fg-faint text-micro">{clock(segment.t0)}</span>
      {/* Two people talking at once. Without this the two chips read as a question and an
          answer, and nothing on screen would tell a reader otherwise. */}
      {overlapping && (
        <span className="text-accent text-micro">{t("transcript.at_the_same_time")}</span>
      )}
    </span>
  );

  const said = (
    <span
      className={cn(
        "mt-0.5 block",
        reading ? "text-body leading-[1.75] font-[var(--font-reading)]" : "text-sm leading-relaxed",
        // A partial is still being revised; showing it as settled text makes the app look like
        // it changes its mind.
        segment.source === "partial" && "text-fg-dim italic",
      )}
    >
      {segment.text}
      {segment.translation && (
        <span
          lang={segment.translation.lang}
          className={cn(
            "text-fg-dim mt-1 block",
            // Italic is how this says "the machine wrote this line". CJK, Thai, Arabic and
            // Hebrew have no italic form, so a browser shears the glyphs instead — harder to
            // read, and it looks like a rendering fault.
            italicise(segment.translation.lang) && "italic",
          )}
        >
          {segment.translation.text}
        </span>
      )}
    </span>
  );

  const className = cn(
    "block w-full rounded-[var(--radius-card)] border px-3 py-2 text-left transition-colors",
    active
      ? "border-accent/40 bg-accent-soft"
      : "border-transparent hover:border-line hover:bg-bg-soft",
    // Indented and ruled, so simultaneous speech is visible before it is read.
    overlapping && "border-l-accent/40 ms-3 rounded-l-none border-l-2",
  );

  // The chip is a container, and what seeks is the line of speech inside it.
  //
  // It used to be one big `<button>`, which is why the speaker's name could not become one: a
  // button inside a button is invalid, and the browser's repair — dropping the inner one out of the
  // parent — leaves a control that is unreachable by keyboard and fires nothing on click. Moving
  // the seek onto the text also stops "who was this?" from being a request to start playback.
  return (
    <div className={className} data-overlapping={overlapping || undefined}>
      {header}
      {seekable ? (
        <button
          type="button"
          className="block w-full text-left"
          onClick={() => onSeek(segment.t0)}
          aria-label={t("meeting.play_from", { time: clock(segment.t0) })}
        >
          {said}
        </button>
      ) : (
        said
      )}
      {open && unnamed && naming && (
        <NameVoice
          voice={unnamed}
          people={naming.people}
          busy={naming.busy === unnamed.label}
          onName={(name) => naming.onName(unnamed.label, name)}
          onCancel={() => onAsk(null)}
        />
      )}
    </div>
  );
}
