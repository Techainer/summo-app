import { useRef } from "react";

import { cn } from "../lib/cn";
import { useT } from "../i18n/context";
import { useVirtualizer } from "@tanstack/react-virtual";
import { formatTime, type Segment } from "../lib/protocol";
import type { Event } from "../lib/protocol";

/**
 * The transcript list.
 *
 * Virtualized because an eight-hour meeting is tens of thousands of lines, and rendering them all
 * is the difference between a smooth scroll and a frozen window. Row height is estimated rather
 * than measured per item; the virtualizer corrects itself as rows render.
 */
export function Transcript({
  segments,
}: {
  segments: Segment[];
  onEvent?: (event: Event) => void;
}) {
  const t = useT();
  const parentRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: segments.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 64,
    overscan: 12,
  });

  return (
    <div ref={parentRef} className="h-full overflow-y-auto px-4 py-3.5">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((row) => {
          const segment = segments[row.index];
          if (!segment) return null;
          return (
            <div
              key={row.key}
              ref={virtualizer.measureElement}
              data-index={row.index}
              className="py-2.5"
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${row.start}px)`,
              }}
            >
              <div className="flex items-baseline gap-2.5">
                <span className="tabular text-[0.76rem] text-fg-faint">
                  {formatTime(segment.t0)}
                </span>
                <span
                  className={cn(
                    "text-[0.8rem] font-semibold",
                    segment.lane === "mic" ? "text-accent" : "text-fg-dim",
                  )}
                >
                  {segment.speaker ?? (segment.lane === "mic" ? t("meeting.speaker_you") : "…")}
                </span>
              </div>
              <p
                className={cn(
                  "mt-0.5 mb-0 leading-relaxed",
                  // Partial text is dimmed rather than hidden, so the eye can follow it without
                  // trusting it yet.
                  segment.source === "partial" ? "text-fg-dim" : "text-fg",
                )}
              >
                {segment.text}
              </p>
              {/* Under the original, never instead of it: the original is what was actually said,
                  and anyone checking a subtitle against the speaker needs both. Dimmed and italic
                  so a glance can tell which line is the machine's. */}
              {segment.translation && (
                <p
                  lang={segment.translation.lang}
                  className="mt-0.5 mb-0 leading-relaxed text-fg-dim italic opacity-[0.72]"
                >
                  {segment.translation.text}
                </p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
