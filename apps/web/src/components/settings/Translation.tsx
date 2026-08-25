import { Check, HardDriveDownload } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button, Input, Progress, Select } from "../ui";
import { CONTROL, FIELD, HINT, LABEL } from "./fields";
import { useT } from "../../i18n/context";
import { CatalogueClient, canRun, size, type CatalogueModel } from "../../lib/catalogue";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { OnboardingClient, POLL_MS, isFinished, type Install } from "../../lib/onboarding";
import { fetchPlan } from "../../lib/plan";
import { useLoad, useRefresh } from "../../lib/use-load";
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
  const { handshake } = useEngine();
  const { llm, edit, save } = settings;

  // What the daemon would translate with, which is not always what this section says.
  //
  // With nothing configured it uses the translation model on disk rather than the summary model —
  // otherwise pressing Install on the models screen bought a model the app then declined to use,
  // and asking for subtitles resolved to the default `ollama` endpoint and failed every line. That
  // resolution has to be visible here, or this section describes a decision the daemon is not
  // making.
  const plan = useLoad(
    useCallback(async () => fetchPlan(handshake), [handshake]),
    [handshake],
  );
  const using = plan.data?.translation.using ?? null;

  if (!llm) return null;

  const mt = llm.translator ?? null;
  const mode = mt === null ? AUTO : mt.provider === LOCAL ? LOCAL : ENDPOINT;

  return (
    <div data-testid="settings-translation">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.mt_hint")}</p>

      {/* One control for one decision, in three states.

          This was a checkbox — "use a separate model for translation" — above a two-option Runs
          dropdown, and the checkbox had become a lie: unticked does not mean "do not use a separate
          model", it means *unset*, and unset is exactly the state in which the daemon now picks the
          translation model on disk. Somebody with SMALL100 installed read an empty box over a
          working translator. The three things `llm.translator` can actually be are the three things
          offered here. */}
      <label className={FIELD}>
        <span className={LABEL}>{t("settings.mt_where")}</span>
        <Select
          className={CONTROL}
          value={mode}
          aria-label={t("settings.mt_where")}
          onChange={(e) =>
            void save({
              ...llm,
              // "In Summo" proposes the in-app model, not an endpoint. "Enable this, now go and
              // install a model server" is not a setting anybody finishes, and the whole claim of
              // this feature is that translation costs nothing — which stops being true the moment
              // it depends on a second program.
              translator:
                e.target.value === AUTO
                  ? null
                  : e.target.value === LOCAL
                    ? { provider: LOCAL, model: "small100" }
                    : { provider: "llama-cpp", model: "milmmt-46-1b" },
            })
          }
        >
          <option value={AUTO}>{t("settings.mt_auto")}</option>
          <option value={LOCAL}>{t("settings.mt_in_app")}</option>
          <option value={ENDPOINT}>{t("settings.mt_endpoint")}</option>
        </Select>
      </label>

      {/* Automatic, resolved out loud. Either the daemon has named a model — say which — or it has
          not, and then translation would fall through to the summary endpoint, which is the failure
          this whole section exists to stop somebody discovering during a meeting. Held back until
          the plan has actually answered: "no translation model" printed over a machine that has one
          is worse than a moment of nothing. */}
      {mode === AUTO &&
        (using ? (
          <p className={cn(HINT, "text-done")}>{t("record.translate_with", { model: using })}</p>
        ) : (
          plan.data !== null && <LocalModel model="small100" missing={t("settings.mt_auto_none")} />
        ))}

      {mode === ENDPOINT && (
        <label className={FIELD}>
          <span className={LABEL}>{t("settings.endpoint")}</span>
          <Input
            className={CONTROL}
            value={mt?.provider ?? ""}
            aria-label={t("settings.mt_endpoint")}
            placeholder="llama-cpp"
            onChange={(e) =>
              edit({
                ...llm,
                translator: { model: mt?.model ?? null, provider: e.target.value },
              })
            }
            onBlur={() => void save(llm)}
          />
        </label>
      )}

      {mode !== AUTO && (
        <>
          <label className={FIELD}>
            <span className={LABEL}>{t("settings.model")}</span>
            <Input
              className={CONTROL}
              value={mt?.model ?? ""}
              aria-label={t("settings.mt_model")}
              placeholder="small100"
              onChange={(e) =>
                edit({
                  ...llm,
                  translator: { provider: mt?.provider ?? LOCAL, model: e.target.value },
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
          {mode === LOCAL ? (
            <LocalModel model={mt?.model ?? null} />
          ) : (
            <p className={HINT}>{t("settings.mt_run")}</p>
          )}
        </>
      )}
    </div>
  );
}

/** `llm.translator == null`: no endpoint and no pinned model, so the daemon decides. */
const AUTO = "auto";
/** Somebody else's server. Not a provider id — the id is whatever they type in the next field. */
const ENDPOINT = "endpoint";

/**
 * Whether the named translation model is on this machine, and a button that puts it there.
 *
 * Turning translation on and getting nothing is the failure this closes: the setting saved happily,
 * the model was never downloaded, and the first translated line arrived as an error during a
 * meeting. Nothing here guesses — the catalogue is asked whether that exact id is installed, and the
 * three answers (have it, downloading it, do not have it) are the three things it can say.
 */
function LocalModel({ model, missing }: { model: string | null; missing?: string }) {
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

  // A model this build has no runtime for. The second install button in the app, and the one most
  // likely to hit this: the release ships the ONNX translation runtime and not llama.cpp, so the
  // two GGUF translators in the registry could be named here, downloaded at 0.8 GB and 2.4 GB, and
  // then refused at the first translated line.
  if (!installed && entry && !canRun(entry)) {
    return <p className={cn(HINT, "text-blocked")}>{entry.why_not}</p>;
  }

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
      {/* Why the button is here at all, when the caller has one to say. In the automatic state
          nothing is configured and nothing is on disk, so the sentence that matters is where the
          translation would go instead — and it is only true while this branch is the one rendering. */}
      {missing && <p className={cn(HINT, "mb-2")}>{missing}</p>}
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
      {/* The whole story rather than a percentage. This said `0%` for the first minute of a 611 MB
          download and somebody reasonably read that as broken. */}
      {running && job && <Progress install={job} className="ml-[162px] max-w-sm" />}
      {job?.state === "failed" && <p className="text-rec text-micro mt-2">{job.error}</p>}
      {error && <p className="text-rec text-micro mt-2">{error}</p>}
    </div>
  );
}
