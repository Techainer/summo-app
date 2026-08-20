import { ArrowDown } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { Avatar } from "./ui";
import { cn } from "../lib/cn";
import { useT } from "../i18n/context";
import { useVirtualizer } from "@tanstack/react-virtual";
import { decorate, italicise } from "../lib/reading";
import { formatTime, type Segment } from "../lib/protocol";
import type { Event } from "../lib/protocol";

/**
 * The transcript list.
 *
 * Virtualized because an eight-hour meeting is tens of thousands of lines, and rendering them all
 * is the difference between a smooth scroll and a frozen window. Row height is estimated rather
 * than measured per item; the virtualizer corrects itself as rows render.
 *
 * The rest of this file is about it being *readable while it is still arriving*, which is a
 * different job from being readable afterwards. See `lib/reading.ts` for the grouping and overlap
 * rules; the two things decided here are scrolling and how an overlap is drawn.
 */
export function Transcript({
  segments,
}: {
  segments: Segment[];
  onEvent?: (event: Event) => void;
}) {
  // Not compiled by the React Compiler, deliberately.
  //
  // `useVirtualizer` keeps a mutable instance and reads layout during render, which the compiler
  // cannot prove is safe to memoise, so it declines the whole component. `"use no memo"` states
  // that; the lint rule reports the skip regardless, so it is silenced here with the reason rather
  // than left to be scrolled past every day. This list is fast because it renders twenty rows out
  // of ten thousand, not because of memoisation — nothing is lost by not compiling it.
  "use no memo";

  const t = useT();
  const parentRef = useRef<HTMLDivElement>(null);
  const rows = decorate(segments);

  /**
   * Whether new lines should pull the view down with them.
   *
   * On until the user scrolls up, and back on when they return to the bottom. A live transcript
   * that always jumps to the newest line makes it impossible to read the line before it — the text
   * moves out from under the eye every few seconds — and one that never jumps stops being live.
   * Neither is a setting; which one is wanted is obvious from where the user is looking.
   */
  const [following, setFollowing] = useState(true);

  const onScroll = useCallback(() => {
    const element = parentRef.current;
    if (!element) return;
    // 48px of slack: a scroll that lands a hair short of the bottom, or a row growing by a line
    // after it renders, should not read as "the user scrolled away".
    const atBottom = element.scrollHeight - element.scrollTop - element.clientHeight < 48;
    setFollowing(atBottom);
  }, []);

  // eslint-disable-next-line react-hooks/incompatible-library -- see the note above the directive
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    // Two lines and a speaker: a translated transcript is roughly this tall, and an estimate that
    // is too small makes the scrollbar jump as rows measure themselves.
    estimateSize: () => 78,
    overscan: 12,
  });

  // `useLayoutEffect`, so the scroll happens in the same frame the row was added. In an effect the
  // browser paints the new line at the old scroll position first, which is a visible jolt.
  useLayoutEffect(() => {
    if (!following || rows.length === 0) return;
    virtualizer.scrollToIndex(rows.length - 1, { align: "end" });
  }, [following, rows.length, virtualizer]);

  // A row that grows — a translation arriving under a line already on screen — moves everything
  // below it. While following, stay at the bottom.
  useEffect(() => {
    if (!following) return;
    const element = parentRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [following, segments]);

  return (
    <div className="relative h-full">
      <div
        ref={parentRef}
        onScroll={onScroll}
        className="h-full overflow-y-auto px-4 py-3.5"
        data-testid="transcript"
      >
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((item) => {
            const row = rows[item.index];
            if (!row) return null;
            const { segment, showSpeaker, overlapping, pause } = row;
            const speaker =
              segment.speaker ?? (segment.lane === "mic" ? t("meeting.speaker_you") : "…");

            return (
              <div
                key={item.key}
                ref={virtualizer.measureElement}
                data-index={item.index}
                className={cn(showSpeaker ? "pt-2.5 pb-1" : "py-1")}
                style={{
                  position: "absolute",
                  top: 0,
                  left: 0,
                  width: "100%",
                  transform: `translateY(${item.start}px)`,
                }}
              >
                {/* Silence is information. Ten seconds between two lines means something happened
                    in the room; running them together loses it. */}
                {pause !== null && (
                  <p className="text-fg-faint text-micro mb-1.5" data-testid="pause">
                    {t("transcript.pause", { seconds: Math.round(pause) })}
                  </p>
                )}

                {showSpeaker && (
                  <div className="flex items-center gap-2.5">
                    <span className="tabular text-fg-faint text-meta">
                      {formatTime(segment.t0)}
                    </span>
                    <Avatar name={speaker} size="sm" />
                    <span
                      className={cn(
                        "text-meta font-semibold",
                        segment.lane === "mic" ? "text-accent" : "text-fg-dim",
                      )}
                    >
                      {speaker}
                    </span>
                  </div>
                )}

                {/* Two people talking at once, drawn as such.

                    One microphone hears both, and the recogniser produces two utterances whose
                    times overlap. Rendered as a plain list they read as a question and an answer,
                    and nothing on screen would tell a reader otherwise — which changes what the
                    meeting appears to have been. The rule is indented and tied to the line above
                    it, and it says so in words for anyone who cannot see the rule. */}
                <div
                  className={cn(overlapping && "border-accent/40 -mt-0.5 border-l-2 ps-2.5")}
                  data-overlapping={overlapping || undefined}
                >
                  {overlapping && (
                    <p className="text-fg-faint text-micro mb-0.5">
                      {t("transcript.at_the_same_time")}
                    </p>
                  )}
                  <p
                    // A stable hook for the browser suites. They used to select `.line-text`, a
                    // class that stopped existing when this component was rewritten — and because
                    // the suite that used it is not part of `pnpm e2e`, nothing said so for weeks.
                    // A `data-testid` is a promise to the tests; a utility class is not.
                    data-testid="transcript-line"
                    data-source={segment.source}
                    className={cn(
                      "mt-0.5 mb-0 leading-relaxed",
                      // Partial text is dimmed rather than hidden, so the eye can follow it
                      // without trusting it yet.
                      segment.source === "partial" ? "text-fg-dim" : "text-fg",
                    )}
                  >
                    {segment.text}
                    {/* A caret on the line still being heard. Dimmed text says "not final" to
                        somebody who already knows the convention; a blinking cursor says "this is
                        happening right now" to everybody, and a screen where words arrive in
                        bursts needs something moving between them. */}
                    {segment.source === "partial" && (
                      <span
                        aria-hidden="true"
                        className="bg-accent ms-1 inline-block h-[1em] w-[2px] translate-y-[0.15em] animate-pulse rounded-full align-baseline"
                      />
                    )}
                  </p>
                  {/* Under the original, never instead of it: the original is what was actually
                      said, and anyone checking a subtitle against the speaker needs both. */}
                  {segment.translation && (
                    <p
                      lang={segment.translation.lang}
                      className={cn(
                        "text-fg-dim mt-0.5 mb-0 leading-relaxed opacity-[0.72]",
                        italicise(segment.translation.lang) && "italic",
                      )}
                    >
                      {segment.translation.text}
                    </p>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Only while the user is reading something older. It is both the way back and the only
          indication that the transcript is still moving down there. */}
      {!following && segments.length > 0 && (
        <button
          type="button"
          onClick={() => setFollowing(true)}
          className="border-line bg-bg-raised text-fg-dim hover:text-fg text-micro absolute inset-x-0 bottom-3 mx-auto flex w-fit items-center gap-1.5 rounded-full border px-3 py-1.5 shadow-[var(--shadow-pop)]"
        >
          <ArrowDown aria-hidden="true" className="size-3.5" />
          {t("transcript.follow")}
        </button>
      )}
    </div>
  );
}
