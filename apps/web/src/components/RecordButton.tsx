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
        "inline-flex items-center gap-2.5 rounded-full border px-4 py-2 font-medium transition-colors",
        recording ? "border-rec bg-bg-soft" : "border-line bg-bg-soft hover:border-fg-faint",
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
          "h-2.5 w-2.5 rounded-full bg-rec",
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
