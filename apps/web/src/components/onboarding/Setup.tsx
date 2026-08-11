import { motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useErrorText } from "../../lib/errors";
import { METER } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import {
  OnboardingClient,
  POLL_MS,
  blocker,
  isFinished,
  needsConsent,
  optional,
  percent,
  preferred,
  size,
  type Install,
  type Recommended,
  type Status,
} from "../../lib/onboarding";
import { Button } from "../ui";

/**
 * The first screen, when there is something in the way.
 *
 * Two rules shape it.
 *
 * **One decision, not four.** Only a missing speech model actually stops a recording; ffmpeg and a
 * language model are listed below as things that unlock features, not as steps to complete. A setup
 * flow that demands four answers before the app does anything is how a first run becomes a bounce.
 *
 * **The reason is shown, not hidden.** Each model carries why it was ranked where it was — "fits in
 * 16 GB", "keeps up with live audio" — so a user can disagree with the choice. A list with no
 * reasons asks them to trust it, and the whole point of a local-first tool is that they do not have
 * to.
 */
export function Setup({ onDone }: { onDone: () => void }) {
  const { handshake } = useEngine();
  const say = useErrorText();
  const { t, locale } = useI18n();
  const client = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const [status, setStatus] = useState<Status | null>(null);
  const [models, setModels] = useState<Recommended[]>([]);
  const [chosen, setChosen] = useState<string | null>(null);
  const [installs, setInstalls] = useState<Install[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [next, running] = await Promise.all([client.status(), client.installs()]);
      setStatus(next);
      setInstalls(running);
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The recommendation is for the language the interface is in: someone who switched to English is
  // very likely recording in English.
  useEffect(() => {
    let cancelled = false;
    client
      .recommend(locale)
      .then((result) => {
        if (cancelled) return;
        setModels(result.models);
        setChosen((current) => current ?? preferred(result.models)?.id ?? null);
      })
      .catch(() => {
        // Offline is a supported state for a local-first tool. The daemon still ranks whatever is
        // installed, so an empty list here means genuinely nothing to offer.
      });
    return () => {
      cancelled = true;
    };
  }, [client, locale]);

  const downloading = installs.some((i) => !isFinished(i));
  useEffect(() => {
    if (!downloading) return undefined;
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [downloading, refresh]);

  const install = async () => {
    if (!chosen) return;
    setError(null);
    try {
      const job = await client.install(chosen);
      setInstalls((current) => [...current.filter((i) => i.model !== job.model), job]);
    } catch (e) {
      setError(say(e));
    }
  };

  const finish = async () => {
    try {
      await client.complete();
    } catch {
      // Failing to write the flag means the checklist appears again, which is annoying but not
      // wrong. It must not stop the user getting into the app.
    }
    onDone();
  };

  if (!status) {
    return <p className="text-fg-faint mt-24 text-center">{t("common.loading")}</p>;
  }

  const stuck = blocker(status);
  const later = optional(status);
  const selected = models.find((m) => m.id === chosen);

  return (
    <div className="mx-auto mt-12 w-full max-w-2xl px-6 pb-16">
      <h1 className="text-2xl font-semibold">{t("setup.title")}</h1>
      <p className="text-fg-dim mt-2">{t("setup.promise")}</p>

      {error && (
        <p role="alert" className="text-danger mt-4 text-sm">
          {error}
        </p>
      )}

      {stuck ? (
        <section className="mt-8">
          <h2 className="text-base font-medium">{t("setup.pick_model")}</h2>
          <p className="text-fg-dim mt-1 text-sm">
            {t("setup.machine", {
              cores: status.hardware.cores,
              ram: Math.round(status.hardware.total_ram_mb / 1024),
            })}
          </p>

          {models.length === 0 ? (
            <p className="text-fg-faint mt-4 text-sm">{t("setup.no_models")}</p>
          ) : (
            <ul className="mt-4 space-y-2">
              {models.map((m) => {
                const job = installs.find((i) => i.model === m.id);
                const pct = job ? percent(job) : null;
                return (
                  <li key={m.id}>
                    <label
                      className={`flex cursor-pointer gap-3 rounded-xl border p-3 ${
                        m.id === chosen ? "border-accent bg-accent-soft" : "border-line bg-bg-soft"
                      }`}
                    >
                      <input
                        type="radio"
                        name="model"
                        value={m.id}
                        checked={m.id === chosen}
                        onChange={() => setChosen(m.id)}
                        className="mt-1"
                      />
                      <span className="min-w-0 flex-1">
                        <span className="flex items-baseline justify-between gap-3">
                          <span className="font-medium">{m.name}</span>
                          <span className="text-fg-faint text-[13px]">
                            {m.installed ? t("setup.installed") : size(m.size_bytes)}
                          </span>
                        </span>
                        <span className="text-fg-dim mt-0.5 block text-[13px]">{m.reason}</span>
                        {m.license && (
                          <span className="text-fg-faint mt-0.5 block text-[12px]">
                            {m.license}
                            {needsConsent(m) ? ` · ${t("setup.upstream")}` : ""}
                          </span>
                        )}
                        {job && (
                          <span className="bg-line mt-2 block h-1 overflow-hidden rounded-full">
                            <motion.span
                              className="bg-accent block h-full"
                              animate={
                                pct === null ? { x: ["-100%", "100%"] } : { width: `${pct}%` }
                              }
                              transition={
                                pct === null
                                  ? {
                                      repeat: Infinity,
                                      duration: 1.2,
                                      ease: "linear",
                                    }
                                  : METER
                              }
                              style={pct === null ? { width: "40%" } : undefined}
                            />
                          </span>
                        )}
                        {job?.error && (
                          <span className="text-danger mt-1 block text-[12px]">{job.error}</span>
                        )}
                      </span>
                    </label>
                  </li>
                );
              })}
            </ul>
          )}

          {selected && needsConsent(selected) && (
            <p className="border-blocked/30 bg-blocked-soft text-blocked mt-3 rounded-xl border p-3 text-[13px]">
              {t("setup.upstream_note")}
            </p>
          )}

          <Button className="mt-4" onClick={() => void install()} disabled={!chosen || downloading}>
            {downloading ? t("setup.downloading") : t("setup.install")}
          </Button>
        </section>
      ) : (
        <section className="border-ok/30 bg-ok-soft mt-8 rounded-xl border p-4">
          <p className="font-medium">{t("setup.ready")}</p>
          <p className="text-fg-dim mt-1 text-sm">{t("setup.ready_hint")}</p>
        </section>
      )}

      {later.length > 0 && (
        <section className="mt-8">
          <h2 className="text-base font-medium">{t("setup.later")}</h2>
          <ul className="text-fg-dim mt-2 space-y-1 text-sm">
            {later.map((check) => (
              <li key={check.step}>
                <b className="text-fg font-medium">{t(`setup.step_${check.step}`)}</b> —{" "}
                {t(`setup.why_${check.step}`)}
              </li>
            ))}
          </ul>
        </section>
      )}

      <div className="mt-10 flex gap-2">
        <Button onClick={() => void finish()} disabled={!status.can_record}>
          {t("setup.start")}
        </Button>
        {/* Available even while blocked: someone who wants to look around before downloading half
            a gigabyte should be able to. It acknowledges as well — a welcome screen that returns on
            every launch after being dismissed is nagging, and the banner still says a model is
            missing. */}
        <Button variant="ghost" onClick={() => void finish()}>
          {t("setup.skip")}
        </Button>
      </div>
    </div>
  );
}
