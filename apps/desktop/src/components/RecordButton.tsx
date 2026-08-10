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
  return (
    <button
      type="button"
      className={recording ? "record recording" : "record"}
      onClick={onToggle}
      aria-pressed={recording}
      aria-label={recording ? "Dừng ghi" : "Bắt đầu ghi"}
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
