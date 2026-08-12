import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { decorate, italicise, type Row } from "../../lib/reading";
import { clock } from "./Player";

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

interface Props {
  segments: Line[];
  /** Playhead position, so the line being spoken can be marked. */
  at?: number;
  onSeek?: (seconds: number) => void;
  /** Reading mode uses Be Vietnam Pro and looser leading, for actually reading it back. */
  reading?: boolean;
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
export function TranscriptChips({ segments, at, onSeek, reading = false }: Props) {
  const t = useT();
  const scroller = useRef<HTMLDivElement>(null);

  // The same rules the recording screen uses. Two views of one transcript that disagreed about
  // who was talking over whom would be two answers to a question with one answer.
  const rows = useMemo(() => decorate(segments), [segments]);

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
              <Chip row={line} active={isActive} reading={reading} onSeek={onSeek} />
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
}: {
  row: Row<Line>;
  active: boolean;
  reading: boolean;
  onSeek?: (seconds: number) => void;
}) {
  const { segment, overlapping, showSpeaker } = row;
  // Unconditionally, at the top. Below the early return this was a hook called only on some
  // renders, which is the one thing React's rules forbid outright — the hook order changes the
  // first time a chip becomes seekable and every hook after it in the tree reads somebody else's
  // state.
  const t = useT();
  const seekable = onSeek !== undefined;

  const body = (
    <>
      <span className="flex items-baseline gap-2">
        {/* Only at the start of a run. Three chips in a row from one person is one turn, and
            repeating the name on each of them is what turns a paragraph into a list — the exact
            thing this component's chips exist to make visible. */}
        {segment.speaker && showSpeaker && (
          <span className="text-fg-dim text-micro font-semibold">{segment.speaker}</span>
        )}
        <span className="tabular text-fg-faint text-micro">{clock(segment.t0)}</span>
        {/* Two people talking at once. Without this the two chips read as a question and an
            answer, and nothing on screen would tell a reader otherwise. */}
        {overlapping && (
          <span className="text-accent text-micro">{t("transcript.at_the_same_time")}</span>
        )}
      </span>
      <span
        className={cn(
          "mt-0.5 block",
          reading
            ? "text-body leading-[1.75] font-[var(--font-reading)]"
            : "text-sm leading-relaxed",
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
    </>
  );

  const className = cn(
    "block w-full rounded-[var(--radius-card)] border px-3 py-2 text-left transition-colors",
    active
      ? "border-accent/40 bg-accent-soft"
      : "border-transparent hover:border-line hover:bg-bg-soft",
    // Indented and ruled, so simultaneous speech is visible before it is read.
    overlapping && "border-l-accent/40 ms-3 rounded-l-none border-l-2",
  );

  if (!seekable) {
    return (
      <div className={className} data-overlapping={overlapping || undefined}>
        {body}
      </div>
    );
  }
  return (
    <button
      type="button"
      className={className}
      data-overlapping={overlapping || undefined}
      onClick={() => onSeek(segment.t0)}
      aria-label={t("meeting.play_from", { time: clock(segment.t0) })}
    >
      {body}
    </button>
  );
}
