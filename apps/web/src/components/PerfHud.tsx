import { X } from "lucide-react";
import { useEffect, useState } from "react";

import { useT } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { POLL_MS, busyParts, isOn, onChange, perf, shareOfRam, show, type Perf } from "../lib/perf";

/**
 * What Summo is costing, in the corner, for whoever wants to know.
 *
 * **Off unless asked for, and closable from itself.** A permanent gauge in the corner of a
 * recorder is an invitation to watch a number instead of a meeting — and a panel you can only turn
 * off by finding the right settings section is a panel people resent. The × here is the same
 * switch as the one in Settings → General.
 *
 * ## Why these numbers and not others
 *
 * The status bar already reports the *machine's* memory. This reports **Summo's**, which is the
 * different question and the one somebody running a background daemon on their laptop is actually
 * asking. Beside it: which models are loaded right now — not which are configured — because a few
 * hundred megabytes resident while nothing is recording is the warm decoder, and that is the whole
 * answer to "why is this using memory when I'm not recording".
 *
 * Nothing is invented. A figure the daemon could not measure is drawn as `—`, never as zero: the
 * first CPU reading has no interval behind it, and a panel that showed 0% for it would report an
 * idle daemon and an unmeasured one identically.
 */
export function PerfHud() {
  const t = useT();
  const { handshake } = useEngine();
  const [on, setOn] = useState(isOn);
  const [reading, setReading] = useState<Perf | null>(null);

  useEffect(() => onChange(setOn), []);

  useEffect(() => {
    if (!on) return undefined;
    let cancelled = false;
    const ask = () => {
      perf(handshake)
        .then((next) => !cancelled && setReading(next))
        .catch(() => undefined);
    };
    ask();
    const timer = window.setInterval(ask, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      // Dropped on the way out, not on the way in. A stale reading redrawn when the panel comes
      // back would be a number from some earlier minute presented as now — and clearing it in the
      // body of the effect is a render-phase write that React is right to object to.
      setReading(null);
    };
  }, [on, handshake]);

  if (!on) return null;

  const share = reading ? shareOfRam(reading) : null;
  const jobs = reading ? busyParts(reading) : [];

  return (
    <aside
      aria-label={t("perf.title")}
      data-testid="perf-hud"
      // Bottom-left, above the status bar, and clear of the sidebar on a wide screen.
      //
      // It was on the right, which is where everything else already is: the assistant panel opens
      // down that side, the home screen's ask bar ends there, and — the one that actually collided
      // — the minimised meeting pins itself to `end-4 bottom-4` at up to 416 pixels wide. Two
      // fixed panels in one corner means the layer decides which you can see, and `docked` losing
      // to `float` is the right answer to the wrong question: the readout should not have been
      // under it at all. `Tour` reached the same corner for the same reason and left the note that
      // said so.
      //
      // `lg:` clears the 210px sidebar; below that breakpoint the sidebar is a sheet and the
      // corner is free.
      className="border-line bg-bg-raised/95 rounded-card text-micro fixed start-3 bottom-12 z-[var(--z-docked)] w-56 border p-3 shadow-[var(--shadow-pop)] backdrop-blur lg:start-[calc(210px+0.75rem)]"
    >
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-fg-dim font-medium">{t("perf.title")}</span>
        <button
          type="button"
          onClick={() => show(false)}
          aria-label={t("perf.hide")}
          className="text-fg-faint hover:text-fg rounded-inline -me-1 -mt-1 p-1 transition-colors"
        >
          <X aria-hidden="true" className="size-3.5" />
        </button>
      </div>

      <dl className="mt-2 space-y-1">
        <Row
          label={t("perf.memory")}
          value={
            reading?.rss_mb == null
              ? "—"
              : share == null
                ? `${reading.rss_mb} MB`
                : `${reading.rss_mb} MB · ${share.toFixed(1)}%`
          }
        />
        <Row
          label={t("perf.cpu")}
          // `null` is "not measured yet", and it is the honest answer for exactly one poll.
          value={reading?.cpu_percent == null ? "—" : `${reading.cpu_percent.toFixed(0)}%`}
        />
        {reading?.recording && reading.audio_s !== null && (
          <Row
            label={t("perf.listening")}
            value={t("perf.listening_value", {
              minutes: Math.floor(reading.audio_s / 60),
              lines: reading.segments ?? 0,
            })}
          />
        )}
      </dl>

      {/* Which models are *loaded*. The answer to "why is this holding memory while idle". */}
      {reading && reading.models.length > 0 && (
        <ul className="border-line mt-2 space-y-0.5 border-t pt-2">
          {reading.models.map((model) => (
            <li key={`${model.role}:${model.id}`} className="flex items-baseline gap-2">
              <span className="text-fg-faint w-14 shrink-0">{t(`perf.role_${model.role}`)}</span>
              <span className="text-fg-dim min-w-0 flex-1 truncate font-mono">{model.id}</span>
            </li>
          ))}
        </ul>
      )}

      {/* Background work, so a busy daemon that is not recording has an explanation on screen
          rather than a number the reader has to account for. */}
      {jobs.length > 0 && (
        <p className="text-fg-faint border-line mt-2 border-t pt-2">
          {jobs.map((job) => t(job.key, { count: job.count })).join(" · ")}
        </p>
      )}

      {reading === null && <p className="text-fg-faint mt-2">{t("perf.waiting")}</p>}
    </aside>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <dt className="text-fg-faint">{label}</dt>
      <dd className="nums tabular text-fg-dim">{value}</dd>
    </div>
  );
}
