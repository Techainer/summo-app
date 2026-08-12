import { cn } from "../lib/cn";
import { useT } from "../i18n/context";
import { formatTime } from "../lib/protocol";

/**
 * The one control that has to be instant.
 *
 * No confirmation, no model picker, no dialog — the promise is that pressing record starts a
 * recording in under a second, so anything that stands between the press and the capture is a bug.
 */
export function RecordButton({
  recording,
  elapsed,
  onToggle,
}: {
  recording: boolean;
  elapsed: number;
  onToggle: () => void;
}) {
  const t = useT();
  return (
    <button
      type="button"
      className={cn(
        "inline-flex items-center gap-2.5 rounded-full border px-4 py-2 font-medium",
        "transition-all duration-200 active:scale-[0.97]",
        // A halo while recording, and only while recording.
        //
        // This control sits in a header the user has navigated away from — the promise is that a
        // meeting is stoppable from anywhere, which is only true if they can *find* it. Colour
        // alone is a 10 px dot; the glow is what makes the state legible from across a desk.
        // Not a substitute for the dot or the timer: it is a third signal for the same fact, which
        // is what a state this consequential deserves.
        recording
          ? "border-rec bg-rec-soft shadow-[0_0_0_3px_var(--color-rec-soft),0_0_20px_-2px_var(--color-rec)]"
          : "border-line bg-bg-soft hover:border-fg-faint hover:bg-bg-raised",
      )}
      onClick={onToggle}
      aria-pressed={recording}
      aria-label={recording ? t("record.stop") : t("record.start")}
    >
      {/* Only pulses while recording: a dot that always throbs stops meaning anything, and this
          is the one state that must never be mistaken for another. */}
      <span
        aria-hidden
        className={cn(
          "bg-rec h-2.5 w-2.5 rounded-full",
          recording && "motion-safe:animate-[pulse_1.6s_ease-in-out_infinite]",
        )}
      />
      {recording ? (
        <span className="tabular">{formatTime(elapsed)}</span>
      ) : (
        <span>{t("record.short")}</span>
      )}
    </button>
  );
}
