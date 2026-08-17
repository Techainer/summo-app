import { m, useReducedMotion } from "motion/react";
import { useEffect, useRef, useState } from "react";

import { cn } from "../../lib/cn";

/**
 * The stickers.
 *
 * A system emoji is a 20-pixel glyph in whatever style the machine happens to ship, and it reads as
 * punctuation beside a heading. These are the animated ones: Google's Noto emoji animations, which
 * are the only sticker set of this quality with a licence that can simply be obeyed — CC-BY 4.0,
 * credited in `public/stickers/LICENSE.md`. Everything else good is licensed per asset on a website,
 * and art with unrecorded provenance is a problem whoever forks this repository inherits.
 *
 * ## What this costs, and what it does not
 *
 * The animations are JSON in `public/`, not imports: they are fetched when a sticker is actually on
 * screen and never enter a chunk. The player is `lottie-web`'s light build, loaded by `import()` at
 * the same moment, so an app that never shows an empty state never pays for either. Nothing here is
 * on the path to the first paint.
 *
 * ## The drawing underneath
 *
 * Each sticker also exists as a few hundred bytes of SVG, drawn in the theme's own colours, and it
 * is what renders first — while the JSON is in flight, when the fetch fails, and under
 * `prefers-reduced-motion`, where an animation is the part nobody asked for. So there is never a
 * blank square where a picture is about to be, and the layout never shifts under one.
 */

export type StickerName = "sprout" | "party" | "wave" | "coffee" | "magnifier" | "robot" | "pencil";

const ACCENT = "var(--color-accent)";
const AI = "var(--color-ai)";
const REC = "var(--color-rec)";
const WARM = "var(--color-blocked)";

interface Props {
  name: StickerName;
  /** Edge of the square it is drawn in. */
  size?: number;
  className?: string;
}

export function Sticker({ name, size = 112, className }: Props) {
  const still = useReducedMotion() ?? false;
  const box = useRef<HTMLDivElement>(null);
  const [playing, setPlaying] = useState(false);
  const Fallback = SCENES[name];

  useEffect(() => {
    // Still means still: no player, no fetch, no animation frame. The drawing says the same thing.
    if (still) return undefined;
    let animation: { destroy: () => void } | null = null;
    let cancelled = false;

    void (async () => {
      try {
        const [{ default: lottie }, data] = await Promise.all([
          import("lottie-web/build/player/lottie_light"),
          fetch(`stickers/${name}.json`).then((r) => {
            if (!r.ok) throw new Error(String(r.status));
            return r.json() as Promise<object>;
          }),
        ]);
        if (cancelled || !box.current) return;
        animation = lottie.loadAnimation({
          container: box.current,
          renderer: "svg",
          loop: true,
          autoplay: true,
          animationData: data,
        });
        setPlaying(true);
      } catch {
        // A sticker that could not be fetched is a sticker that stays drawn. This is decoration on
        // a screen with nothing on it; it is not worth an error anybody reads.
      }
    })();

    return () => {
      cancelled = true;
      animation?.destroy();
      setPlaying(false);
    };
  }, [name, still]);

  return (
    <div
      aria-hidden="true"
      // `data-ink` for the reason `Spot` carries it: `e2e/density.mjs` decides whether a screen is a
      // composition or a hole by walking for text and a handful of tag names, and a drawing is
      // neither.
      data-ink=""
      className={cn("relative shrink-0 select-none", className)}
      style={{ width: size, height: size }}
    >
      {/* Underneath, always drawn, hidden the moment the animation is actually playing — so the
          swap is one opacity change rather than a box that pops into existence. */}
      <div
        className={cn(
          "absolute inset-0 transition-opacity duration-300",
          playing ? "opacity-0" : "opacity-100",
        )}
      >
        <svg viewBox="0 0 120 120" width={size} height={size} fill="none">
          <Fallback still={still} />
        </svg>
      </div>
      <div ref={box} className="absolute inset-0" style={{ width: size, height: size }} />
    </div>
  );
}

interface SceneProps {
  still: boolean;
}

/** A slow bob, shared by the drawings whose whole body moves. */
const bob = (still: boolean, distance = 3, seconds = 4) =>
  still
    ? undefined
    : {
        animate: { y: [0, -distance, 0] },
        transition: { duration: seconds, repeat: Infinity, ease: "easeInOut" as const },
      };

/** A pot with something new in it: the vault with nothing in it yet. */
function Sprout({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={ACCENT} opacity="0.10" />
      <m.g {...bob(still, 2.5, 5)}>
        <path d="M60 84 V54" stroke={ACCENT} strokeWidth="4" strokeLinecap="round" />
        <path d="M60 62 C46 62 40 54 40 46 C52 46 60 52 60 62 Z" fill={ACCENT} opacity="0.85" />
        <path d="M60 68 C74 68 80 60 80 52 C68 52 60 58 60 68 Z" fill={AI} opacity="0.7" />
      </m.g>
      <path d="M42 84 H78 L74 104 H46 Z" fill={WARM} opacity="0.9" />
      <rect x="38" y="78" width="44" height="9" rx="4.5" fill={WARM} />
    </>
  );
}

/** Confetti, for a board with nothing left on it. */
function Party({ still }: SceneProps) {
  const bits = [
    { x: 30, y: 26, c: ACCENT, r: 12 },
    { x: 86, y: 32, c: AI, r: -20 },
    { x: 24, y: 62, c: WARM, r: 40 },
    { x: 94, y: 68, c: REC, r: -35 },
    { x: 58, y: 18, c: AI, r: 8 },
  ];
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={AI} opacity="0.10" />
      <m.g {...bob(still, 2, 3.6)}>
        <path d="M40 96 L72 60 L84 72 L48 104 Z" fill={ACCENT} opacity="0.9" />
        <path d="M72 60 L84 72 L78 78 L66 66 Z" fill={ACCENT} />
      </m.g>
      {bits.map((bit, at) => (
        <rect
          key={at}
          x={bit.x}
          y={bit.y}
          width="9"
          height="9"
          rx="2.5"
          fill={bit.c}
          transform={`rotate(${bit.r} ${bit.x + 4.5} ${bit.y + 4.5})`}
          opacity="0.8"
        />
      ))}
    </>
  );
}

/** A hand, waving. The first thing a new vault says. */
function Wave({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={ACCENT} opacity="0.10" />
      <m.g
        style={{ originX: "60px", originY: "96px" }}
        animate={still ? undefined : { rotate: [0, 14, -5, 12, 0] }}
        transition={{ duration: 2.8, repeat: Infinity, repeatDelay: 1.2, ease: "easeInOut" }}
      >
        <rect x="42" y="56" width="36" height="46" rx="16" fill={WARM} />
        <rect x="46" y="34" width="8" height="30" rx="4" fill={WARM} />
        <rect x="56" y="28" width="8" height="36" rx="4" fill={WARM} />
        <rect x="66" y="32" width="8" height="32" rx="4" fill={WARM} />
        <rect x="74" y="44" width="8" height="24" rx="4" fill={WARM} opacity="0.95" />
        <rect
          x="34"
          y="60"
          width="8"
          height="20"
          rx="4"
          fill={WARM}
          transform="rotate(-24 38 70)"
        />
      </m.g>
    </>
  );
}

/** A cup with steam off it: nothing is waiting on you. */
function Coffee({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={WARM} opacity="0.12" />
      {[46, 60, 74].map((x, at) => (
        <m.path
          key={x}
          d={`M${x} 44 C${x - 5} 36 ${x + 5} 32 ${x} 24`}
          stroke={ACCENT}
          strokeWidth="3.5"
          strokeLinecap="round"
          opacity="0.5"
          animate={still ? undefined : { y: [0, -5, 0], opacity: [0.25, 0.6, 0.25] }}
          transition={{ duration: 3.2, repeat: Infinity, ease: "easeInOut", delay: at * 0.5 }}
        />
      ))}
      <path d="M34 54 H80 V76 C80 86 72 94 62 94 H52 C42 94 34 86 34 76 Z" fill={ACCENT} />
      <path
        d="M80 60 H88 C93 60 96 64 96 69 C96 74 93 78 88 78 H80"
        stroke={ACCENT}
        strokeWidth="5"
        strokeLinecap="round"
      />
      <rect x="30" y="96" width="54" height="7" rx="3.5" fill={ACCENT} opacity="0.35" />
    </>
  );
}

/** A magnifier that found nothing. */
function Magnifier({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={AI} opacity="0.10" />
      <m.g
        style={{ originX: "60px", originY: "60px" }}
        animate={still ? undefined : { rotate: [0, -8, 6, 0], x: [0, 3, -2, 0] }}
        transition={{ duration: 6, repeat: Infinity, ease: "easeInOut" }}
      >
        <circle cx="54" cy="52" r="24" stroke={AI} strokeWidth="7" />
        <circle cx="54" cy="52" r="24" fill={AI} opacity="0.10" />
        <path d="M72 70 L92 90" stroke={AI} strokeWidth="9" strokeLinecap="round" />
        <path
          d="M44 44 C46 39 50 36 55 35"
          stroke="white"
          strokeWidth="4"
          strokeLinecap="round"
          opacity="0.8"
        />
      </m.g>
    </>
  );
}

/** The agent, idle. */
function Robot({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={AI} opacity="0.10" />
      <m.g {...bob(still, 3, 4.4)}>
        <rect x="34" y="44" width="52" height="42" rx="14" fill={AI} opacity="0.9" />
        <circle cx="50" cy="62" r="5.5" fill="white" />
        <circle cx="70" cy="62" r="5.5" fill="white" />
        <rect x="52" y="74" width="16" height="4" rx="2" fill="white" opacity="0.65" />
        <path d="M60 44 V34" stroke={AI} strokeWidth="4" strokeLinecap="round" />
        <circle cx="60" cy="30" r="5" fill={ACCENT} />
        <rect x="28" y="56" width="8" height="18" rx="4" fill={AI} opacity="0.6" />
        <rect x="84" y="56" width="8" height="18" rx="4" fill={AI} opacity="0.6" />
      </m.g>
    </>
  );
}

/** A pencil on a page: nothing written yet. */
function Pencil({ still }: SceneProps) {
  return (
    <>
      <circle cx="60" cy="60" r="46" fill={ACCENT} opacity="0.10" />
      <rect x="32" y="30" width="46" height="60" rx="8" fill="white" opacity="0.85" />
      <rect
        x="32"
        y="30"
        width="46"
        height="60"
        rx="8"
        stroke={ACCENT}
        strokeWidth="3"
        opacity="0.5"
      />
      <rect x="41" y="48" width="28" height="4" rx="2" fill={ACCENT} opacity="0.35" />
      <rect x="41" y="58" width="28" height="4" rx="2" fill={ACCENT} opacity="0.35" />
      <rect x="41" y="68" width="16" height="4" rx="2" fill={ACCENT} opacity="0.35" />
      <m.g
        style={{ originX: "78px", originY: "78px" }}
        animate={still ? undefined : { rotate: [0, -6, 0], x: [0, 2, 0] }}
        transition={{ duration: 4.6, repeat: Infinity, ease: "easeInOut" }}
      >
        <path d="M64 92 L88 44 L98 50 L74 98 Z" fill={WARM} />
        <path d="M64 92 L74 98 L63 100 Z" fill={ACCENT} />
      </m.g>
    </>
  );
}

const SCENES: Record<StickerName, (props: SceneProps) => React.ReactElement> = {
  sprout: Sprout,
  party: Party,
  wave: Wave,
  coffee: Coffee,
  magnifier: Magnifier,
  robot: Robot,
  pencil: Pencil,
};
