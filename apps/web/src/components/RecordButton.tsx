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
      className={recording ? "record recording" : "record"}
      onClick={onToggle}
      aria-pressed={recording}
      aria-label={recording ? t("record.stop") : t("record.start")}
    >
      <span className="record-dot" aria-hidden />
      {recording ? (
        <span className="record-time">{formatTime(elapsed)}</span>
      ) : (
        <span>Ghi</span>
      )}
    </button>
  );
}
