import { AudioLines } from "lucide-react";
import { AnimatePresence, m } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import {
  DubClient,
  POLL_MS,
  describe,
  isFinished,
  percent,
  voiceToPull,
  voicesFor,
} from "../../lib/dub";
import { CatalogueClient } from "../../lib/catalogue";
import { ExportClient } from "../../lib/export";
import { GENTLE, METER, listItem } from "../../lib/motion";
import { size } from "../../lib/catalogue";
import { useInstall } from "../../lib/use-install";
import { useLoad, useRefresh } from "../../lib/use-load";
import { Button, Card, CardBody, CardHeader, Progress } from "../ui";

/**
 * The meeting, spoken in another language, over its own recording.
 *
 * The pipeline has worked for several releases and nothing could reach it. `summo dub` existed, had
 * tests, and two voices were published for it — and there was no route, no screen and not one
 * string in the catalogue, so from inside the app the feature did not exist. This is the door.
 *
 * ## What it refuses to offer
 *
 * A language with no translation, and a language with no voice installed that speaks it. Both are
 * checked here *and* in the daemon, which is not duplication — the daemon's check is the one that
 * is true (it is the only one that can see the disk at the moment the job starts), and this one
 * exists so the button is never offered for work that cannot happen. A control that is offered and
 * then refuses is worse than one that says why it is missing.
 *
 * The reason the voice has to match is not politeness. A VITS voice handed a language it was not
 * trained for does not fail: it runs the text through the phoneme table it has and speaks the
 * result, confidently, over somebody's meeting.
 */
interface Props {
  meeting: string;
  /** False for a typed note: there is no recording to speak over. */
  recorded: boolean;
  /** Called when a dub finishes, so the player can pick up the new track. */
  onDone?: () => void;
}

export function Dub({ meeting, recorded, onDone }: Props) {
  const { t, locale } = useI18n();
  const { handshake } = useEngine();
  const say = useErrorText();
  const client = useMemo(() => new DubClient(handshake), [handshake]);
  const exports = useMemo(() => new ExportClient(handshake), [handshake]);
  const catalogue = useMemo(() => new CatalogueClient(handshake), [handshake]);

  const [jobs, setJobs] = useState<Awaited<ReturnType<DubClient["list"]>>>([]);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState("");

  /** The languages this meeting has been translated into — the only ones there is text to speak. */
  const translated = useLoad(
    useCallback(
      async () => (recorded ? await exports.languages(meeting) : []),
      [exports, meeting, recorded],
    ),
    [exports, meeting, recorded],
  );

  /** What is on this machine. A voice that is not installed is not an option, it is a download. */
  const models = useLoad(
    useCallback(async () => (recorded ? await catalogue.installed() : []), [catalogue, recorded]),
    [catalogue, recorded],
  );

  /**
   * And what the registry has, so a missing voice can be named and fetched from here.
   *
   * Failing quietly is right: a registry nobody can reach costs the offer below, not the panel.
   */
  const shop = useLoad(
    useCallback(async () => {
      if (!recorded) return [];
      try {
        return (await catalogue.load()).models;
      } catch {
        return [];
      }
    }, [catalogue, recorded]),
    [catalogue, recorded],
  );

  /** A download started from this panel. */
  const install = useInstall(handshake);

  const refresh = useCallback(async () => {
    try {
      const all = await client.list();
      setJobs(all.filter((job) => job.meeting === meeting));
    } catch {
      // A daemon that is briefly busy is not a reason to blank a list somebody is reading.
    }
  }, [client, meeting]);

  useRefresh(refresh);

  const busy = jobs.some((job) => !isFinished(job));
  useEffect(() => {
    if (!busy) return undefined;
    const timer = window.setInterval(() => void refresh(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [busy, refresh]);

  /**
   * Tell the page once, when a dub lands.
   *
   * On the count of finished jobs rather than on `jobs`, which is a new array every poll — the
   * meeting would be refetched every two seconds for as long as this card is on screen.
   */
  const finished = jobs.filter((job) => job.state === "done").length;
  useEffect(() => {
    if (finished > 0) onDone?.();
  }, [finished, onDone]);

  const nameOf = useMemo(() => {
    const names = new Intl.DisplayNames([locale], { type: "language" });
    return (code: string) => {
      try {
        return names.of(code) ?? code;
      } catch {
        return code;
      }
    };
  }, [locale]);

  const start = (lang: string) => {
    setStarting(lang);
    setError(null);
    void (async () => {
      try {
        const job = await client.start(meeting, lang);
        setJobs((current) => [job, ...current]);
      } catch (e) {
        setError(say(e));
      } finally {
        setStarting("");
      }
    })();
  };

  if (!recorded) return null;

  const languages = translated.data ?? [];
  const voices = models.data ?? [];
  /** A language is dubbable when there is text to speak and something installed that can say it. */
  const canSpeak = languages.filter((lang) => voicesFor(voices, lang).length > 0);
  const noVoice = languages.filter((lang) => voicesFor(voices, lang).length === 0);
  const running = new Set(jobs.filter((job) => !isFinished(job)).map((job) => job.lang));

  // Nothing translated and nothing running: the way in is the translate row above this card, and
  // saying so beats an empty card that looks broken.
  if (languages.length === 0 && jobs.length === 0) {
    return (
      <Card>
        <CardHeader title={t("dub.title")} count={t("dub.subtitle")} />
        <CardBody>
          <p className="text-fg-dim text-meta">{t("dub.needs_translation")}</p>
        </CardBody>
      </Card>
    );
  }

  const said = (job: (typeof jobs)[number]) => {
    const described = describe(job);
    return "text" in described ? described.text : t(described.key, described.values);
  };

  return (
    <Card>
      <CardHeader title={t("dub.title")} count={t("dub.subtitle")} />
      <CardBody className="space-y-3">
        {canSpeak.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <AudioLines aria-hidden="true" className="text-fg-faint me-0.5 size-3.5" />
            <span className="text-fg-faint text-micro me-1">{t("dub.speak_in")}</span>
            {canSpeak.map((lang) => (
              <Button
                key={lang}
                size="sm"
                variant="ghost"
                busy={starting === lang}
                disabled={running.has(lang)}
                onClick={() => start(lang)}
              >
                {nameOf(lang)}
              </Button>
            ))}
          </div>
        )}

        {/* Translated, and nothing installed can say it. Named rather than silently dropped: the
            difference between "Summo cannot do this" and "you need a voice for this" is the whole
            of whether the reader knows what to press next, and the models screen is where. */}
        {/* Translated, and nothing installed can say it.

            This named the languages and then sent the reader to another screen — in the one place
            somebody has already decided they want a dub, with the fix a click away and unnamed.
            The registry has a voice for Vietnamese, English and Chinese; the panel says which,
            how big, and fetches it here. Same shape as the recognition panel's offer, and the
            same reason: a dead end in a feature's own screen is the feature not existing. */}
        {noVoice.map((lang) => {
          const voice = voiceToPull(shop.data ?? [], lang);
          return (
            <div
              key={lang}
              className="border-accent/30 bg-accent-soft text-meta rounded-control border px-3 py-2"
              data-testid="dub-needs-voice"
            >
              <p className="text-fg-dim">
                {voice
                  ? t("dub.needs_voice", {
                      language: nameOf(lang),
                      voice: voice.name,
                      size: size(voice.size_bytes),
                    })
                  : t("dub.no_voice", { languages: nameOf(lang) })}
              </p>
              {voice && (
                <div className="mt-2">
                  <Button
                    size="sm"
                    busy={install.running}
                    onClick={() => {
                      void install.start(voice.id).then((ok) => {
                        if (ok) models.reload();
                      });
                    }}
                  >
                    {t("dub.install_voice")}
                  </Button>
                </div>
              )}
              {install.job && <Progress install={install.job} />}
            </div>
          );
        })}

        <ul className="space-y-2">
          <AnimatePresence initial={false}>
            {jobs.map((job) => {
              const pct = percent(job);
              return (
                <m.li
                  key={job.id}
                  variants={listItem}
                  initial="hidden"
                  animate="shown"
                  exit="gone"
                  transition={GENTLE}
                  className="border-line bg-bg-soft rounded-card border p-3"
                >
                  <div className="flex items-baseline justify-between gap-3">
                    <span className="text-body min-w-0 truncate font-medium">
                      {nameOf(job.lang)}
                    </span>
                    <span
                      className={
                        job.state === "failed" ? "text-danger text-meta" : "text-fg-dim text-meta"
                      }
                    >
                      {said(job)}
                    </span>
                  </div>

                  {!isFinished(job) && (
                    <div
                      className="bg-line mt-2 h-1 overflow-hidden rounded-full"
                      role="progressbar"
                      aria-valuenow={pct ?? undefined}
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-label={t("dub.progress_label", { language: nameOf(job.lang) })}
                    >
                      {/* Length unknown: an indeterminate sweep. A bar frozen at 0% is the one
                          thing a job that runs for minutes must not look like. */}
                      <m.div
                        className="bg-accent h-full"
                        animate={pct === null ? { x: ["-100%", "100%"] } : { width: `${pct}%` }}
                        transition={
                          pct === null ? { repeat: Infinity, duration: 1.2, ease: "linear" } : METER
                        }
                        style={pct === null ? { width: "40%" } : undefined}
                      />
                    </div>
                  )}

                  {/* How well it fitted. A line that runs past its gap is the one thing a dub can
                      get wrong that nobody notices until they listen to the whole thing, so the
                      count is on screen rather than only in the log. */}
                  {job.state === "done" && (job.overflowing ?? 0) > 0 && (
                    <p className="text-fg-dim text-micro mt-2">
                      {t("dub.overflowing", {
                        count: job.overflowing ?? 0,
                        seconds: (job.worst_over_s ?? 0).toFixed(1),
                      })}
                    </p>
                  )}
                </m.li>
              );
            })}
          </AnimatePresence>
        </ul>

        {error && (
          <p role="alert" className="text-danger text-meta">
            {error}
          </p>
        )}
      </CardBody>
    </Card>
  );
}
