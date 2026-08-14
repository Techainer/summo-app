import { cn } from "../../lib/cn";

/**
 * A recording, drawn as bars.
 *
 * Rows in the library said "42 phút · Bạn, Ngọc" and nothing else, so a page of recordings was a
 * page of identical text. A waveform gives each one a silhouette — a long monologue and a fast
 * back-and-forth do not look alike — which is what makes a list scannable rather than readable.
 *
 * **The shape is derived from the id, not from the audio.** Decoding a meeting to draw a thumbnail
 * would mean reading megabytes to paint sixty pixels, and the summaries the library renders from do
 * not carry an envelope. So this is an *ornament that is stable per recording*: the same meeting
 * always draws the same bars, two meetings look different, and nothing about it claims to be a
 * measurement. When the vault starts storing a real envelope, `bars` takes it and nothing else here
 * changes.
 *
 * `aria-hidden`, because it says nothing a screen reader could use. Duration and speakers are text
 * beside it.
 */
export function Wave({
  seed,
  bars = 28,
  className,
  live = false,
  breathe = false,
  levels,
}: {
  /** Anything stable per recording — the id. Same seed, same silhouette. */
  seed: string;
  bars?: number;
  className?: string;
  /** Animate, for something being recorded right now. */
  live?: boolean;
  /**
   * Breathe slowly while nothing is happening.
   *
   * Only for the one large waveform on the capture card. Idle, it was forty static grey bars filling
   * a third of the home screen, which reads as a picture of a broken meter rather than as an
   * invitation. Four seconds and a shallow amplitude is slow enough that it is never the thing you
   * are looking at, and alive enough that the card is not a dead rectangle.
   *
   * Never set on a list: a page of library rows each animating forty bars is forty rows of
   * compositor work for an ornament nobody is watching.
   */
  breathe?: boolean;
  /** Real levels in [0,1], when there are some. Overrides the seeded shape. */
  levels?: number[];
}) {
  const heights = levels ?? shape(seed, bars);
  const moving = live || breathe;

  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex h-full w-full items-center justify-between gap-[2px] overflow-hidden",
        className,
      )}
    >
      {heights.map((height, at) => (
        <span
          key={at}
          className={cn(
            // A fixed narrow bar, not `w-full`: stretched across a thousand pixels the bars became
            // 14px wide, and a 14px bar with a full round is a dot. Three pixels reads as sound at
            // any width, and `justify-between` spreads them.
            "w-[3px] shrink-0 rounded-[2px] bg-current opacity-70",
            live && "animate-[wave_1.1s_ease-in-out_infinite]",
            breathe && !live && "animate-[breathe_4s_ease-in-out_infinite]",
          )}
          style={{
            height: `${Math.max(8, height * 100)}%`,
            // Neighbouring bars must not rise together or the whole thing pulses like a heartbeat.
            // Idle, the offsets are spread much wider so the movement travels along the bar rather
            // than flickering in place.
            animationDelay: moving ? `${(at % (live ? 7 : bars)) * (live ? 90 : 70)}ms` : undefined,
          }}
        />
      ))}
    </span>
  );
}

/**
 * Bar heights in [0.08, 1] from a string.
 *
 * A hash, then a small oscillator on top of it: pure hashing gives white noise, which reads as a
 * barcode rather than as speech. The sine puts slow swells under the randomness, which is what an
 * envelope of people talking actually looks like.
 */
function shape(seed: string, bars: number): number[] {
  let hash = 0x811c9dc5;
  for (const point of seed) {
    hash ^= point.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }

  const out: number[] = [];
  for (let at = 0; at < bars; at += 1) {
    hash = (Math.imul(hash, 1664525) + 1013904223) >>> 0;
    const noise = (hash >>> 8) / 0xffffff;
    const swell = 0.55 + 0.45 * Math.sin((at / bars) * Math.PI * 2.7 + (hash % 100) / 40);
    out.push(Math.min(1, Math.max(0.08, noise * 0.55 + swell * 0.55)));
  }
  return out;
}
