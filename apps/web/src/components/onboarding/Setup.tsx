import { Check, Cpu, Languages, MonitorSmartphone, ShieldCheck } from "lucide-react";
import { m } from "motion/react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { useErrorText } from "../../lib/errors";
import { METER, listItem, stagger } from "../../lib/motion";
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
import { Button, Card, CardBody, PageGlow, Sticker } from "../ui";
import { load as loadCapture, save as saveCapture } from "../../lib/capture";
import { languageName, rememberLanguage } from "../../lib/languages";
import { Permissions } from "./Permissions";
import { useRefresh } from "../../lib/use-load";

/**
 * Languages offered first: the ones the interface itself speaks, which is who is reading this.
 *
 * Not a ranking of importance — a ranking of likelihood, and one that stays honest because the
 * second list underneath holds everything else the registry can serve.
 */
const SPOKEN_FIRST = ["vi", "en", "ja", "zh"];

/**
 * The rest, in the order Whisper's own language list has them, which is roughly by speakers.
 *
 * Trimmed to the ones a user is plausibly recording a meeting in. The full ninety-nine are still
 * reachable — the record bar's picker lists every language the daemon reports — but a first-run
 * screen with a hundred-item dropdown is a first-run screen nobody finishes.
 */
const OTHER_SPOKEN = [
  "ko",
  "th",
  "id",
  "ms",
  "fr",
  "de",
  "es",
  "pt",
  "ru",
  "hi",
  "ta",
  "it",
  "nl",
  "pl",
  "tr",
  "ar",
  "sv",
  "da",
  "fi",
  "no",
  "cs",
  "el",
  "he",
  "uk",
  "ro",
  "hu",
];

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

  // The language being *spoken*, which is the question this screen used to answer by assuming it
  // matched the interface. It starts from the interface locale — a reasonable first guess — and is
  // asked out loud, because the cost of the guess being wrong is a download that cannot transcribe
  // the meeting it was installed for.
  const [spoken, setSpoken] = useState(() => loadCapture().spoken || locale);
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

  useRefresh(refresh);

  // The recommendation follows the language above. Changing it re-ranks the models, because "which
  // model" and "which language" are one decision: gipformer is the best answer for Vietnamese and
  // no answer at all for Japanese.
  useEffect(() => {
    let cancelled = false;
    client
      .recommend(spoken)
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
  }, [client, spoken]);

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
    saveCapture({ ...loadCapture(), spoken });
    // The daemon's copy, for every client that is not this browser: the tray, the CLI, a second
    // profile. Ignored on failure — a preference that would not save must not block the app.
    await rememberLanguage(handshake, spoken).catch(() => undefined);
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

  // Which numbered card is which depends on the build. A binary with no recognition has no language
  // question and no model to choose, so numbering the permission card "3" there would be counting
  // two steps that are not on the screen.
  const asks = status.recognition;

  return (
    <div className="relative h-full overflow-y-auto">
      <PageGlow />
      <m.div
        initial="hidden"
        animate="shown"
        transition={stagger(5)}
        className="mx-auto w-full max-w-2xl px-5 pt-10 pb-16 sm:px-8"
      >
        {/* The greeting, and the one sentence that is the entire argument for this product.

            A drawing beside it rather than above it: the first screen of an app is where a person
            decides whether it was built by people who care, and a bare heading on an empty page is
            the same first impression the rest of this interface was redrawn to stop making. */}
        <m.header variants={listItem} className="flex items-start gap-4 sm:gap-5">
          <Sticker name="wave" size={84} className="hidden sm:block" />
          <div className="min-w-0 flex-1">
            <Sticker name="wave" size={56} className="mb-3 sm:hidden" />
            <h1 className="text-display font-semibold tracking-tight text-balance">
              {t("setup.title")}
            </h1>
            <p className="text-fg-dim mt-2 text-pretty">{t("setup.promise")}</p>
            <p className="border-accent/25 bg-accent-soft text-accent text-micro mt-3 inline-flex items-center gap-1.5 rounded-[var(--radius-pill)] border px-2.5 py-1 font-medium">
              <ShieldCheck aria-hidden="true" className="size-3.5" />
              {t("setup.local")}
            </p>
          </div>
        </m.header>

        {error && (
          <m.p
            variants={listItem}
            role="alert"
            className="border-rec/30 bg-rec-soft text-rec text-meta mt-6 rounded-[var(--radius-card)] border px-3 py-2"
          >
            {error}
          </m.p>
        )}

        {/* Asked before the models, because it decides them. A picker below a list of models would
            be a question asked after the answer — and not asked at all in a build with no
            recognition, where it would decide nothing and its own hint would be describing a list
            that is not there. */}
        {asks && (
          <Step index={1} icon={Languages} title={t("setup.spoken")} done>
            <label className="mt-1 block">
              <span className="sr-only">{t("setup.spoken")}</span>
              <select
                value={spoken}
                aria-label={t("setup.spoken")}
                onChange={(event) => setSpoken(event.target.value)}
                className="border-line bg-bg-soft text-fg hover:border-line-strong focus-visible:border-accent h-10 w-full rounded-[var(--radius-card)] border px-3 text-sm transition-colors focus:outline-none sm:w-auto sm:min-w-56"
              >
                {/* The interface languages first — the overwhelmingly likely answers — then
                    everything the registry can serve, which is where a Japanese speaker reading
                    Vietnamese finds their language. */}
                {SPOKEN_FIRST.map((code) => (
                  <option key={code} value={code}>
                    {languageName(code, locale)}
                  </option>
                ))}
                {models.length > 0 && <option disabled>──────────</option>}
                {OTHER_SPOKEN.filter((code) => !SPOKEN_FIRST.includes(code)).map((code) => (
                  <option key={code} value={code}>
                    {languageName(code, locale)}
                  </option>
                ))}
              </select>
            </label>
            <p className="text-fg-dim text-meta mt-2">{t("setup.spoken_hint")}</p>
          </Step>
        )}

        {/* A build that cannot transcribe says so, instead of selling a catalogue.

            `--no-models` is a real shape — the small tarball, and any build on a platform ONNX
            Runtime has no binaries for — and until this was here the screen offered models, took
            the download, and left the user with hundreds of megabytes and a recording that refused
            to start. There is nothing to choose on this screen in that build; the way out is a
            different download, not a different model. */}
        {stuck && !asks ? (
          <m.section variants={listItem} className="mt-6">
            <Card>
              <CardBody className="pt-5">
                <h2 className="text-body font-semibold">{t("setup.no_recognition")}</h2>
                <p className="text-fg-dim mt-1.5 text-sm">{t("setup.no_recognition_hint")}</p>
              </CardBody>
            </Card>
          </m.section>
        ) : stuck ? (
          <Step index={2} icon={Cpu} title={t("setup.pick_model")} done={false}>
            <p className="text-fg-dim text-meta">
              {t("setup.machine", {
                cores: status.hardware.cores,
                ram: Math.round(status.hardware.total_ram_mb / 1024),
              })}
            </p>

            {models.length === 0 ? (
              <p className="text-fg-faint mt-4 text-sm">{t("setup.no_models")}</p>
            ) : (
              <ul className="mt-3 space-y-2">
                {models.map((model) => {
                  const job = installs.find((i) => i.model === model.id);
                  const pct = job ? percent(job) : null;
                  const picked = model.id === chosen;
                  return (
                    <li key={model.id}>
                      {/* A native radio, styled rather than replaced. `accent-color` gives the
                          platform's own control in the theme's colour, which keeps the focus ring,
                          the arrow keys and the screen-reader announcement that a hand-drawn
                          indicator would have had to reimplement. */}
                      <label
                        className={cn(
                          "flex cursor-pointer gap-3 rounded-[var(--radius-card)] border p-3 transition-colors",
                          picked
                            ? "border-accent bg-accent-soft"
                            : "border-line bg-bg-soft hover:border-line-strong",
                        )}
                      >
                        <input
                          type="radio"
                          name="model"
                          value={model.id}
                          checked={picked}
                          onChange={() => setChosen(model.id)}
                          className="mt-1 size-4 shrink-0 [accent-color:var(--color-accent)]"
                        />
                        <span className="min-w-0 flex-1">
                          <span className="flex items-baseline justify-between gap-3">
                            <span className="font-medium">{model.name}</span>
                            <span className="text-fg-faint text-meta shrink-0">
                              {model.installed ? t("setup.installed") : size(model.size_bytes)}
                            </span>
                          </span>
                          <span className="text-fg-dim text-meta mt-0.5 block">{model.reason}</span>
                          {model.license && (
                            <span className="text-fg-faint text-micro mt-0.5 block">
                              {model.license}
                              {needsConsent(model) ? ` · ${t("setup.upstream")}` : ""}
                            </span>
                          )}
                          {job && (
                            <span className="bg-line mt-2 block h-1 overflow-hidden rounded-full">
                              <m.span
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
                            <span className="text-danger text-micro mt-1 block">{job.error}</span>
                          )}
                        </span>
                      </label>
                    </li>
                  );
                })}
              </ul>
            )}

            {selected && needsConsent(selected) && (
              <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta mt-3 rounded-[var(--radius-card)] border p-3">
                {t("setup.upstream_note")}
              </p>
            )}

            <Button
              className="mt-4"
              onClick={() => void install()}
              disabled={!chosen || downloading}
            >
              {downloading ? t("setup.downloading") : t("setup.install")}
            </Button>
          </Step>
        ) : (
          <m.section variants={listItem} className="mt-6">
            <div className="border-accent/30 bg-accent-soft flex items-start gap-3 rounded-[var(--radius-card)] border p-4">
              <span className="bg-accent text-accent-fg mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full">
                <Check aria-hidden="true" className="size-4" />
              </span>
              <span>
                <span className="block font-medium">{t("setup.ready")}</span>
                <span className="text-fg-dim mt-1 block text-sm">{t("setup.ready_hint")}</span>
              </span>
            </div>
          </m.section>
        )}

        {/* After the model, before the "later" list. The model is the one thing that blocks a
            recording *inside* Summo; the microphone is the one thing that blocks it outside, and a
            first run that installs 73 MB and then fails on a permission has wasted the download and
            the trust. Compact here: the system-audio note belongs in Settings, not in the way of
            somebody who has not recorded anything yet. */}
        <Step
          index={asks ? 3 : 1}
          icon={MonitorSmartphone}
          title={t("setup.permission")}
          done={false}
          bare
        >
          <Permissions compact />
        </Step>

        {later.length > 0 && (
          <m.section variants={listItem} className="mt-6">
            <h2 className="text-fg-faint text-micro font-medium tracking-wide uppercase">
              {t("setup.later")}
            </h2>
            <ul className="text-fg-dim mt-2 space-y-1 text-sm">
              {later.map((check) => (
                <li key={check.step}>
                  <b className="text-fg font-medium">{t(`setup.step_${check.step}`)}</b> —{" "}
                  {t(`setup.why_${check.step}`)}
                </li>
              ))}
            </ul>
          </m.section>
        )}

        <m.div variants={listItem} className="mt-8 flex flex-wrap gap-2">
          <Button onClick={() => void finish()} disabled={!status.can_record}>
            {t("setup.start")}
          </Button>
          {/* Available even while blocked: someone who wants to look around before downloading half
              a gigabyte should be able to. It acknowledges as well — a welcome screen that returns
              on every launch after being dismissed is nagging, and the banner still says a model is
              missing. */}
          <Button variant="ghost" onClick={() => void finish()}>
            {t("setup.skip")}
          </Button>
        </m.div>
      </m.div>
    </div>
  );
}

/**
 * One numbered thing to do.
 *
 * The screen used to be four headings and a rule between them, which is a document rather than a
 * flow: nothing said how many decisions there were, which one was answered, or whether the third
 * was even required. A number in a circle answers all three at a glance, and a tick replacing it
 * says the answer has been given.
 *
 * `bare` is for a card that already draws its own rows — the permission panel — so it gets the
 * number and the heading without a second border around what it puts inside.
 */
function Step({
  index,
  icon: Icon,
  title,
  done,
  bare = false,
  children,
}: {
  index: number;
  icon: typeof Cpu;
  title: string;
  done: boolean;
  bare?: boolean;
  children: ReactNode;
}) {
  return (
    <m.section variants={listItem} className="mt-6">
      <div className="mb-2.5 flex items-center gap-2.5">
        <span
          className={cn(
            "text-micro flex size-6 shrink-0 items-center justify-center rounded-full font-semibold",
            done ? "bg-accent text-accent-fg" : "bg-bg-raised border-line text-fg-dim border",
          )}
        >
          {done ? <Check aria-hidden="true" className="size-3.5" /> : index}
        </span>
        <Icon aria-hidden="true" className="text-fg-faint size-4" />
        <h2 className="text-body font-semibold tracking-tight">{title}</h2>
      </div>
      {bare ? (
        children
      ) : (
        <Card>
          <CardBody className="pt-4">{children}</CardBody>
        </Card>
      )}
    </m.section>
  );
}
