import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { isFinished, percent, size, type Install } from "../../lib/onboarding";
import { perSecond, roughly, useProgress } from "../../lib/use-progress";

/**
 * A download, said in full: how far, how fast, and how long is left.
 *
 * Every place that installs a model used to draw a spinning button and, at best, a percentage. On a
 * 611 MB file over a slow link that is a number which moves once every thirty seconds beside an
 * animation that moves constantly — so the animation is what a person reads, and what it says is
 * "something is happening and I cannot tell you what". Two users in a row reported an install as
 * hung. It was downloading the whole time.
 *
 * The bar answers "how far", the byte counts answer "how far in real terms", and the rate and the
 * estimate answer the actual question, which is whether to keep waiting or go and do something
 * else. None of it is new information — the daemon has reported `done` and `total` since installs
 * existed — it was simply never shown.
 */
export function Progress({ install, className }: { install: Install; className?: string }) {
  const t = useT();
  const { rate, eta } = useProgress(install);

  if (isFinished(install)) return null;

  const done = percent(install);
  const left = roughly(eta);

  return (
    <div className={cn("mt-2", className)} data-testid="install-progress">
      {/* Indeterminate until the daemon reports a total: a bar sitting at zero while a request
          negotiates TLS is the same lie as the spinner it replaces. */}
      <div className="bg-bg-soft h-1.5 w-full overflow-hidden rounded-full">
        <div
          role="progressbar"
          aria-valuenow={done ?? undefined}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-label={install.name}
          className={cn(
            "bg-accent h-full rounded-full",
            done === null
              ? "w-1/3 motion-safe:animate-[indeterminate_1.4s_ease-in-out_infinite]"
              : "transition-[width] duration-500",
          )}
          {...(done === null ? {} : { style: { width: `${done}%` } })}
        />
      </div>

      <p className="text-fg-dim text-micro nums mt-1.5">
        {install.state === "installing"
          ? t("models.unpacking")
          : done === null
            ? t("models.starting")
            : [
                // The percentage leads because it is the fastest thing to read and the bar carries
                // no number. The byte counts follow because on a 611 MB file "3%" is the part that
                // means nothing: 18.3 MB of 611 MB is what tells somebody this is a long wait
                // rather than a stuck one.
                `${done}%`,
                install.total
                  ? t("models.of_total", {
                      done: size(install.done ?? 0),
                      total: size(install.total),
                    })
                  : "",
                perSecond(rate),
                left ? t(`models.left_${left.unit}`, { value: String(left.value) }) : "",
              ]
                .filter(Boolean)
                .join(" · ")}
      </p>
    </div>
  );
}
