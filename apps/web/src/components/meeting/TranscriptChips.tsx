import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef } from "react";

import { cn } from "../../lib/cn";
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
  const scroller = useRef<HTMLDivElement>(null);

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
    return <p className="p-6 text-center text-fg-faint">Không có transcript.</p>;
  }

  return (
    <div ref={scroller} className="h-full overflow-y-auto px-1">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((row) => {
          const segment = segments[row.index];
          if (!segment) return null;
          const isActive = row.index === active;
          return (
            <div
              key={row.key}
              ref={virtualizer.measureElement}
              data-index={row.index}
              className="absolute inset-x-0 top-0 pb-2"
              style={{ transform: `translateY(${row.start}px)` }}
            >
              <Chip
                segment={segment}
                active={isActive}
                reading={reading}
                onSeek={onSeek}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function Chip({
  segment,
  active,
  reading,
  onSeek,
}: {
  segment: Line;
  active: boolean;
  reading: boolean;
  onSeek?: (seconds: number) => void;
}) {
  const seekable = onSeek !== undefined;

  const body = (
    <>
      <span className="flex items-baseline gap-2">
        {segment.speaker && (
          <span className="text-[12px] font-semibold text-fg-dim">{segment.speaker}</span>
        )}
        <span className="tabular text-[11px] text-fg-faint">{clock(segment.t0)}</span>
      </span>
      <span
        className={cn(
          "mt-0.5 block",
          reading ? "font-[var(--font-reading)] text-[15px] leading-[1.75]" : "text-sm leading-relaxed",
          // A partial is still being revised; showing it as settled text makes the app look like
          // it changes its mind.
          segment.source === "partial" && "text-fg-dim italic",
        )}
      >
        {segment.text}
        {segment.translation && (
          <span
            lang={segment.translation.lang}
            className="mt-1 block italic text-fg-dim"
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
  );

  if (!seekable) {
    return <div className={className}>{body}</div>;
  }
  return (
    <button
      type="button"
      className={className}
      onClick={() => onSeek(segment.t0)}
      aria-label={`Nghe từ ${clock(segment.t0)}`}
    >
      {body}
    </button>
  );
}
