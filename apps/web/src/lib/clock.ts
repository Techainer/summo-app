/**
 * A running time, as people write it.
 *
 * Shared rather than repeated: the player, the record panel and the home screen all show elapsed
 * time, and two of them had grown their own copy of this with different rules about the hour mark.
 */
/** `h:mm:ss` once past an hour, `m:ss` before it — a 40-minute meeting should not read `0:40:12`. */
export function clock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
