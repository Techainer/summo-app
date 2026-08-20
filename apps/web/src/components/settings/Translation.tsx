import { Check, HardDriveDownload } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button, Checkbox, Input } from "../ui";
import { CONTROL, FIELD, HINT, LABEL, SELECT } from "./fields";
import { useT } from "../../i18n/context";
import { CatalogueClient, size, type CatalogueModel } from "../../lib/catalogue";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { OnboardingClient, POLL_MS, isFinished, percent, type Install } from "../../lib/onboarding";
import { useRefresh } from "../../lib/use-load";
import { LOCAL, type LlmSettings } from "./llm";

/**
 * The model that translates, which is not the model that summarises.
 *
 * A 1B translation model beats a general 8B one at translating, runs on the CPU in under a second a
 * line, and costs nothing — which is what lets somebody with no API key at all still translate
 * every meeting they record. Its own section for the same reason it is its own model: turning
 * translation on is a different decision from choosing who writes your summaries.
 */
export function Translation({ settings }: { settings: LlmSettings }) {
  const t = useT();
  const { llm, edit, save } = settings;
  if (!llm) return null;

  return (
    <div data-testid="settings-translation">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.mt_hint")}</p>

      <Checkbox
        className="mt-3.5"
        checked={llm.translator != null}
        onChange={(on) =>
          void save({
            ...llm,
            // Turning it on proposes the in-app model, not an endpoint. "Enable this, now go and
            // install a model server" is not a setting anybody finishes, and the whole claim of
            // this feature is that translation costs nothing — which stops being true the moment it
            // depends on a second program.
            translator: on ? { provider: LOCAL, model: "small100" } : null,
          })
        }
      >
        {t("settings.mt_enable")}
      </Checkbox>

      {llm.translator != null && (
        <>
          <label className={FIELD}>
            <span className={LABEL}>{t("settings.mt_where")}</span>
            <select
              className={SELECT}
              value={llm.translator.provider === LOCAL ? LOCAL : "endpoint"}
              aria-label={t("settings.mt_where")}
              onChange={(e) =>
                void save({
                  ...llm,
                  translator:
                    e.target.value === LOCAL
                      ? { provider: LOCAL, model: "small100" }
                      : { provider: "llama-cpp", model: "milmmt-46-1b" },
                })
              }
            >
              <option value={LOCAL}>{t("settings.mt_in_app")}</option>
              <option value="endpoint">{t("settings.mt_endpoint")}</option>
            </select>
          </label>

          {llm.translator.provider !== LOCAL && (
            <label className={FIELD}>
              <span className={LABEL}>{t("settings.endpoint")}</span>
              <Input
                className={CONTROL}
                value={llm.translator.provider}
                aria-label={t("settings.mt_endpoint")}
                placeholder="llama-cpp"
                onChange={(e) =>
                  edit({
                    ...llm,
                    translator: { model: llm.translator?.model ?? null, provider: e.target.value },
                  })
                }
                onBlur={() => void save(llm)}
              />
            </label>
          )}

          <label className={FIELD}>
            <span className={LABEL}>{t("settings.model")}</span>
            <Input
              className={CONTROL}
              value={llm.translator.model ?? ""}
              aria-label={t("settings.mt_model")}
              placeholder="small100"
              onChange={(e) =>
                edit({
                  ...llm,
                  translator: {
                    provider: llm.translator?.provider ?? LOCAL,
                    model: e.target.value,
                  },
                })
              }
              onBlur={() => void save(llm)}
            />
          </label>

          {/* The model, installed here. This said "Cài một lần bằng `summo pull small100` — 611 MB"
              to somebody using a desktop app: a terminal command, a model id and a size, in place of
              the one thing they wanted, which was to have the model. Then it became a link to the
              catalogue, which is better and still asks somebody who has just turned translation on
              to go to another screen, find one row among ten and press the button on it. */}
          {llm.translator.provider === LOCAL ? (
            <LocalModel model={llm.translator.model} />
          ) : (
            <p className={HINT}>{t("settings.mt_run")}</p>
          )}
        </>
      )}
    </div>
  );
}

/**
 * Whether the named translation model is on this machine, and a button that puts it there.
 *
 * Turning translation on and getting nothing is the failure this closes: the setting saved happily,
 * the model was never downloaded, and the first translated line arrived as an error during a
 * meeting. Nothing here guesses — the catalogue is asked whether that exact id is installed, and the
 * three answers (have it, downloading it, do not have it) are the three things it can say.
 */
function LocalModel({ model }: { model: string | null }) {
  const t = useT();
  const { handshake } = useEngine();
  const say = useErrorText();
  const catalogue = useMemo(() => new CatalogueClient(handshake), [handshake]);
  const installer = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const [entry, setEntry] = useState<CatalogueModel | null | undefined>(undefined);
  const [job, setJob] = useState<Install | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!model) return;
    try {
      const { models } = await catalogue.load();
      setEntry(models.find((each) => each.id === model) ?? null);
    } catch {
      // The catalogue being unreachable is not this section's story to tell — the models screen
      // says it plainly. Here it means the state is unknown, and offering the button is the answer
      // that still gets somebody a model when the list is merely slow.
      setEntry(null);
    }
  }, [catalogue, model]);

  useRefresh(load);

  // Only while something is running, and it stops on its own when the download ends.
  useEffect(() => {
    if (!job || isFinished(job)) return undefined;
    const timer = window.setInterval(() => {
      void installer
        .installs()
        .then((running) => {
          const mine = running.find((each) => each.model === model);
          if (mine) setJob(mine);
          if (mine && isFinished(mine)) void load();
        })
        .catch(() => undefined);
    }, POLL_MS);
    return () => window.clearInterval(timer);
  }, [job, installer, model, load]);

  if (!model) return null;

  const installed = entry?.installed === true;
  const running = job !== null && !isFinished(job);
  const done = job ? percent(job) : null;

  if (installed) {
    return (
      <p className={cn(HINT, "text-done flex items-center gap-1.5")}>
        <Check aria-hidden="true" className="size-3.5" />
        {t("settings.mt_ready", { model })}
      </p>
    );
  }

  return (
    <div>
      <Button
        variant="secondary"
        size="sm"
        busy={running}
        onClick={() => {
          void installer
            .install(model)
            .then((started) => {
              setJob(started);
              setError(null);
            })
            .catch((e) => setError(say(e)));
        }}
      >
        <HardDriveDownload aria-hidden="true" className="me-1.5 size-3.5" />
        {t("settings.mt_pull")}
        {entry && entry.size_bytes > 0 && ` · ${size(entry.size_bytes)}`}
      </Button>
      {running && (
        <p className={cn(HINT, "nums")}>{done === null ? t("models.starting") : `${done}%`}</p>
      )}
      {job?.state === "failed" && <p className="text-rec text-micro mt-2">{job.error}</p>}
      {error && <p className="text-rec text-micro mt-2">{error}</p>}
    </div>
  );
}
