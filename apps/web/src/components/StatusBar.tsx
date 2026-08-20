import { cn } from "../lib/cn";
import { useI18n } from "../i18n/context";
import type { Memory } from "../lib/memory";
import type { ConnectionState } from "../lib/engine";

/**
 * The performance and connection readout.
 *
 * Unusual to show a user, and deliberate: an app claiming to run recognition locally should prove it
 * is keeping up rather than ask to be trusted. A real-time factor at or above 1.0 means the pipeline
 * is falling behind, which is worth saying plainly instead of degrading silently.
 */
export function StatusBar({
  stat,
  speakers,
  notice,
  connection,
  device,
  memory,
}: {
  stat: { rtf: number; rss_mb: number; queue_ms: number } | null;
  speakers: string[];
  notice: string | null;
  connection: ConnectionState;
  device: string | null;
  memory: Memory | null;
}) {
  // `n`, not `t`: English needs "1 speaker" and "2 speakers"; Vietnamese needs one form for both.
  const { t, n } = useI18n();
  const behind = stat !== null && (stat.rtf >= 1 || stat.queue_ms > 3000);
  const disconnected = connection === "reconnecting" || connection === "connecting";

  return (
    <footer className="border-line text-fg-faint text-micro flex items-center gap-2.5 border-t px-4 py-2">
      {notice && <span className="text-fg-dim">{notice}</span>}
      <span className="flex-1" />
      {disconnected && (
        <span
          className="border-line nums border-rec text-rec text-micro inline-flex items-center rounded-full border px-2 py-0.5"
          data-testid="connection"
        >
          {connection === "reconnecting" ? t("status.reconnecting") : t("status.connecting")}
        </span>
      )}
      {device && (
        <span className="border-line nums text-micro inline-flex items-center rounded-full border px-2 py-0.5">
          {device}
        </span>
      )}
      {speakers.length > 0 && (
        <span className="border-line nums text-micro inline-flex items-center rounded-full border px-2 py-0.5">
          {n("status.speakers", speakers.length)}
        </span>
      )}
      {/* This machine's memory, whether or not anything is recording.

          Requested after three releases spent on a bug whose cause was this number: a 24 GB MacBook
          reported zero bytes free, every model was ranked as too large to run, and the figure that
          decided it was nowhere on screen. Hidden on a phone, where the row has no room and the
          operating system shows it anyway. */}
      {memory && (
        <span
          className="border-line tabular text-micro hidden items-center rounded-full border px-2 py-0.5 sm:inline-flex"
          data-testid="memory"
        >
          RAM {memory.usedGb.toFixed(1)}/{memory.totalGb.toFixed(0)} GB
        </span>
      )}
      {stat && (
        <>
          <span
            className={cn(
              "border-line tabular text-micro inline-flex items-center rounded-full border px-2 py-0.5",
              behind && "border-rec text-rec",
            )}
          >
            RTF {stat.rtf.toFixed(3)}
          </span>
          <span className="border-line tabular text-micro inline-flex items-center rounded-full border px-2 py-0.5">
            {stat.rss_mb} MB
          </span>
          {behind && (
            <span className="border-line nums border-rec text-rec text-micro inline-flex items-center rounded-full border px-2 py-0.5">
              {t("status.behind")}
            </span>
          )}
        </>
      )}
    </footer>
  );
}
