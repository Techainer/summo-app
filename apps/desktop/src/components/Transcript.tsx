import { useRef } from "react";
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
  const parentRef = useRef<HTMLDivElement>(null);

  const virtualizer = useVirtualizer({
    count: segments.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 64,
    overscan: 12,
  });

  return (
    <div ref={parentRef} className="transcript">
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((row) => {
          const segment = segments[row.index];
          if (!segment) return null;
          return (
            <div
              key={row.key}
              ref={virtualizer.measureElement}
              data-index={row.index}
              className={`line ${segment.source}`}
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                width: "100%",
                transform: `translateY(${row.start}px)`,
              }}
            >
              <div className="line-meta">
                <span className="time">{formatTime(segment.t0)}</span>
                <span className={`speaker lane-${segment.lane}`}>
                  {segment.speaker ?? (segment.lane === "mic" ? "Bạn" : "…")}
                </span>
              </div>
              <p className="line-text">{segment.text}</p>
            </div>
          );
        })}
      </div>
    </div>
  );
}
