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
  const behind = stat !== null && (stat.rtf >= 1 || stat.queue_ms > 3000);
  const disconnected = connection === "reconnecting" || connection === "connecting";

  return (
    <footer className="status">
      {notice && <span className="notice">{notice}</span>}
      <span className="spacer" />
      {disconnected && (
        <span className="chip warn" data-testid="connection">
          {connection === "reconnecting" ? "đang kết nối lại…" : "đang kết nối…"}
        </span>
      )}
      {device && <span className="chip">{device}</span>}
      {speakers.length > 0 && <span className="chip">{speakers.length} người nói</span>}
      {stat && (
        <>
          <span className={behind ? "chip warn" : "chip"}>RTF {stat.rtf.toFixed(3)}</span>
          <span className="chip">{stat.rss_mb} MB</span>
          {behind && <span className="chip warn">đang chậm hơn thời gian thực</span>}
        </>
      )}
    </footer>
  );
}
