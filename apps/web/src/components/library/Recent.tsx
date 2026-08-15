import { m } from "motion/react";
import { NotebookPen } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Avatar, Skeleton, Wave } from "../ui";
import { useI18n, useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { formatDuration } from "../../lib/duration";
import { useEngine } from "../../lib/engine-context";
import { LibraryClient, dayLabel, localDay, type MeetingSummary } from "../../lib/library";
import { listItem, stagger } from "../../lib/motion";

/**
 * The last few things in the vault, as cards.
 *
 * Written once and used on the home screen, on the vault screen with nothing selected, and under
 * the record panel — the three places where the honest answer to "what is here" is the recordings
 * themselves. It was written twice before this and had already started to drift: one copy showed
 * the time of day, the other did not.
 *
 * Each card carries the three things that let somebody recognise a recording without reading it: a
 * waveform that differs per recording, the faces of who was in it, and how long it went on.
 */
export function Recent({
  limit = 6,
  columns = 3,
  onOpen,
  className,
}: {
  limit?: number;
  columns?: 2 | 3;
  onOpen: (entry: MeetingSummary) => void;
  className?: string;
}) {
  const t = useT();
  const { locale } = useI18n();
  const { handshake } = useEngine();
  const library = useMemo(() => new LibraryClient(handshake), [handshake]);
  const [entries, setEntries] = useState<MeetingSummary[] | null>(null);

  useEffect(() => {
    let cancelled = false;
    library
      .view({ group: "day" })
      .then((view) => {
        if (cancelled) return;
        setEntries(view.groups.flatMap((group) => group.meetings).slice(0, limit));
      })
      // Silence here is deliberate: this is a strip under the real content of three different
      // screens, and each of them already reports a dead daemon in its own way. A second copy of
      // the same error, three times, is noise.
      .catch(() => setEntries([]));
    return () => {
      cancelled = true;
    };
  }, [library, limit]);

  const grid = cn(
    "grid gap-2",
    columns === 3 ? "sm:grid-cols-2 xl:grid-cols-3" : "sm:grid-cols-2",
    className,
  );

  if (entries === null) {
    return (
      <ul className={grid} aria-hidden="true">
        {Array.from({ length: Math.min(limit, columns) }, (_, at) => (
          <li key={at}>
            <Skeleton className="h-[68px] w-full" />
          </li>
        ))}
      </ul>
    );
  }

  if (entries.length === 0) return null;

  return (
    <m.ul initial="hidden" animate="shown" transition={stagger(entries.length)} className={grid}>
      {entries.map((entry) => (
        <m.li key={entry.id} variants={listItem}>
          {/* A plain button. The lift used to be `whileHover={{ y: -2 }}`, which is Motion running a
              spring on one hover — a rAF loop and an inline transform per card, to move something
              two pixels. `.lift` is the same movement as a CSS transition, it is the same rule every
              other card on every other screen uses, and it costs nothing while the pointer is
              elsewhere. */}
          <button
            type="button"
            onClick={() => onOpen(entry)}
            className="border-line bg-bg-soft lift flex w-full items-center gap-3 rounded-[var(--radius-card)] border p-2.5 text-left"
          >
            {/* A note has no waveform because it was typed; it gets the pen instead, at the same
                size, so the rows still line up. */}
            <span
              className={cn(
                "bg-bg-raised ring-line grid size-11 shrink-0 place-items-center rounded-[var(--radius-card)] ring-1",
                entry.kind === "note" ? "text-fg-faint" : "text-accent/60 p-1.5",
              )}
            >
              {entry.kind === "note" ? (
                <NotebookPen aria-hidden="true" className="size-4" />
              ) : (
                <Wave seed={entry.id} bars={9} />
              )}
            </span>

            <span className="min-w-0 flex-1">
              <span className="text-body block truncate font-medium">{entry.title}</span>
              <span className="text-fg-faint text-micro block truncate">
                {dayLabel(entry.day, localDay(), {
                  locale,
                  today: t("library.today"),
                  yesterday: t("library.yesterday"),
                  week: t("library.week"),
                  unfiled: t("library.unfiled_group"),
                })}
                {entry.kind === "meeting" && entry.duration > 0 && (
                  <> · {formatDuration(entry.duration, locale, "short")}</>
                )}
              </span>
            </span>

            {entry.participants.length > 0 && (
              <span className="flex shrink-0 -space-x-1.5">
                {entry.participants.slice(0, 2).map((who) => (
                  <Avatar key={who} name={who} size="sm" className="ring-bg-soft ring-2" />
                ))}
              </span>
            )}
          </button>
        </m.li>
      ))}
    </m.ul>
  );
}
