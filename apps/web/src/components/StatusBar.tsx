import { cn } from "../lib/cn";
import { useI18n } from "../i18n/context";
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
}: {
  stat: { rtf: number; rss_mb: number; queue_ms: number } | null;
  speakers: string[];
  notice: string | null;
  connection: ConnectionState;
  device: string | null;
}) {
  // `n`, not `t`: English needs "1 speaker" and "2 speakers"; Vietnamese needs one form for both.
  const { t, n } = useI18n();
  const behind = stat !== null && (stat.rtf >= 1 || stat.queue_ms > 3000);
  const disconnected = connection === "reconnecting" || connection === "connecting";

  return (
    <footer className="border-line text-fg-faint flex items-center gap-2.5 border-t px-4 py-2 text-[0.78rem]">
      {notice && <span className="text-fg-dim">{notice}</span>}
      <span className="flex-1" />
      {disconnected && (
        <span
          className="border-line tabular border-rec text-rec inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]"
          data-testid="connection"
        >
          {connection === "reconnecting" ? t("status.reconnecting") : t("status.connecting")}
        </span>
      )}
      {device && (
        <span className="border-line tabular inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]">
          {device}
        </span>
      )}
      {speakers.length > 0 && (
        <span className="border-line tabular inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]">
          {n("status.speakers", speakers.length)}
        </span>
      )}
      {stat && (
        <>
          <span
            className={cn(
              "border-line tabular inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]",
              behind && "border-rec text-rec",
            )}
          >
            RTF {stat.rtf.toFixed(3)}
          </span>
          <span className="border-line tabular inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]">
            {stat.rss_mb} MB
          </span>
          {behind && (
            <span className="border-line tabular border-rec text-rec inline-flex items-center rounded-full border px-2 py-0.5 text-[0.78rem]">
              {t("status.behind")}
            </span>
          )}
        </>
      )}
    </footer>
  );
}
