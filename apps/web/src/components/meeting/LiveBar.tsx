import { Square } from "lucide-react";
import { m } from "motion/react";

import { Button } from "../ui";
import { SessionControls } from "../record/SessionControls";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { Waveform } from "../Waveform";

/**
 * The band across the top of a meeting in progress.
 *
 * A recording used to be announced by a red pill in the window's header, forty pixels wide, on a
 * screen otherwise identical to a note nobody is recording. People asked whether it was running.
 * Three things answer that, and all three are here rather than scattered around the frame:
 *
 * **A dot that pulses.** Position and colour are what the eye reads first; movement is what it
 * reads before either, which is why every camera ever made has one of these.
 *
 * **The clock, at a size you can read across a desk.** Elapsed time is proof of duration — a
 * recording that says 00:00 four minutes in is a recording that is not happening.
 *
 * **The level meter.** The other half of the question is not "is it recording" but "can it hear
 * me", and those fail separately: a muted microphone records silence very reliably. The meter moves
 * with the room, so a person can talk and watch it answer.
 *
 * Stopping is here too, because this is where somebody looks when they want to stop.
 */
export function LiveBar() {
  const t = useT();
  const { elapsed, level, stop, session } = useEngine();

  const minutes = Math.floor(elapsed / 60);
  const seconds = elapsed % 60;
  const clock = `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;

  return (
    <m.div
      initial={{ opacity: 0, y: -8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 260, damping: 24 }}
      data-testid="live-bar"
      className="border-rec/30 bg-rec-soft rounded-[var(--radius-card)] border px-3.5 py-2.5"
    >
      <div className="flex items-center gap-3">
        {/* Two circles: a solid dot and a ring expanding out of it. The ring is what carries at the
          edge of vision — a dot that only changes opacity reads as a static bullet point. */}
        <span
          aria-hidden="true"
          className="relative flex size-3 shrink-0 items-center justify-center"
        >
          <span className="bg-rec absolute inline-flex size-3 rounded-full" />
          <span className="bg-rec/60 absolute inline-flex size-3 rounded-full motion-safe:animate-ping" />
        </span>

        <div className="min-w-0">
          <p className="text-rec text-meta leading-none font-medium">{t("record.recording_now")}</p>
          <p className="text-fg-dim text-micro mt-1 truncate leading-none">
            {session.deviceLabel ?? t("record.microphone")}
          </p>
        </div>

        {/* The meter takes the space that is left. It is the widest thing here on purpose: it is the
          only part that moves with the room rather than with the clock. */}
        {/* A height, because the bars are sized by percentage: a flex child with no height of its own
          collapses, and the meter draws nothing. */}
        <div className="h-8 min-w-0 flex-1">
          <Waveform level={level} active />
        </div>

        <p
          aria-label={t("record.elapsed")}
          className="text-rec nums shrink-0 text-xl leading-none font-semibold tabular-nums"
        >
          {clock}
        </p>

        <Button size="sm" variant="danger" onClick={stop} className="shrink-0">
          <Square aria-hidden="true" className="me-1.5 size-3" />
          {t("record.stop")}
        </Button>
      </div>

      {/* What it is hearing, and everything that can still be changed about it — inside the
          meeting, over the transcript it governs.

          These controls existed only in the shell's banner, which is drawn above the page. So on
          the meeting page the settings for the meeting were outside the meeting, in the app's
          chrome next to the nudges; and that banner has a dismiss button, so one click removed the
          only way to change the model, the language or the translation for the rest of the
          recording. `ListeningIn` yields to this one on this page, so there is exactly one. */}
      <div className="border-rec/20 mt-2.5 border-t pt-2">
        <SessionControls quiet expanded />
      </div>
    </m.div>
  );
}
