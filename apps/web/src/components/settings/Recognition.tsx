import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { useI18n } from "../../i18n/context";
import { CatalogueClient, size } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import {
  autoAvailable,
  betterFor,
  fetchLanguages,
  languageName,
  ordered,
  rememberLanguage,
} from "../../lib/languages";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { Button, SegmentedControl, Select } from "../ui";

/**
 * Which model does the listening, and in which language.
 *
 * The app used to answer this one way round only. You picked a *language*, it ranked the installed
 * models for it and used the winner — the right default, and not a choice. Somebody who wanted
 * Whisper anyway, for a meeting that will switch languages or to compare the two, had no way to
 * say so.
 *
 * Both directions live here, as one choice with two positions:
 *
 * - **By language** — the old behaviour, unchanged and still the default. Pick what will be
 *   spoken; the best installed model for it is used.
 * - **By model** — pick the model first, then any language *it* supports. For Whisper that is
 *   ninety-nine of them, which is the whole reason somebody installs it.
 *
 * Nothing new was needed underneath. `settings.models.live` has always been honoured ahead of the
 * ranking — see `chosen_model` in `server.rs`, which returns early when the setting names a model —
 * and `settings.models.language` has always been the language. This screen is the first thing that
 * writes both deliberately, so the pair says what it means.
 *
 * ## Nothing is ever swapped on your behalf
 *
 * The ranking recommends; it does not act. This panel used to describe a language by the best model
 * that *exists* for it, so a machine holding only Whisper was told "Gipformer sẽ nghe" — naming a
 * model it did not have and was never going to run. The recording engine was right all along and
 * used Whisper; only the description was wrong, and it read as though the app were about to change
 * models by itself.
 *
 * So there are two facts and they are kept apart. `serving` is what will actually hear you, and it
 * is what the panel states. `model` is the best that exists, and when it is meaningfully better it
 * becomes an *offer* — both accuracy figures, the download size, and a button. Declining leaves
 * everything exactly as it was, and changing language never re-points the model: `resolve_models`
 * returns early whenever `settings.models.live` names one.
 *
 * The five-point floor on the gap is in `betterFor`. Advice worth less than that costs more
 * attention than it saves.
 */
export function Recognition() {
  const { t, locale } = useI18n();
  const say = useErrorText();
  const navigate = useNavigate();
  const { handshake } = useEngine();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** Bumped after a write, so both lists are re-read rather than assumed. */
  const [generation, setGeneration] = useState(0);

  const catalogue = useLoad(
    useCallback(
      async () => new CatalogueClient(handshake).load(),
      // eslint-disable-next-line react-hooks/exhaustive-deps -- re-read after a write.
      [handshake, generation],
    ),
    [handshake, generation],
  );
  const languages = useLoad(
    useCallback(
      async () => fetchLanguages(handshake),
      // eslint-disable-next-line react-hooks/exhaustive-deps -- re-read after a write.
      [handshake, generation],
    ),
    [handshake, generation],
  );

  /**
   * Every speech model the registry offers, installed or not.
   *
   * This filtered to `installed` and that was the whole of the "I cannot select whisper-tiny" bug:
   * the model was in the catalogue, visible on the models screen, one click from being downloaded —
   * and absent from the only control that selects one, with nothing on screen to say why. A picker
   * that silently omits the thing you are looking for is worse than one that lists it and tells you
   * it needs 153 MB first.
   *
   * The uninstalled ones are still offered, marked with their size, and selecting one says what to
   * do rather than pinning a model the daemon cannot load.
   */
  const speech = useMemo(
    () =>
      (catalogue.data?.models ?? [])
        .filter((model) => model.task === "asr" && model.langs.length > 0)
        .sort((a, b) => {
          if (a.installed !== b.installed) return a.installed ? -1 : 1;
          return a.size_bytes - b.size_bytes;
        }),
    [catalogue.data],
  );

  const current = languages.data?.current ?? "";
  // Which model the `live` role points at. `/catalogue` reports it because the models screen needs
  // the same fact; `/languages` deliberately does not — it answers "what would serve this
  // language", which is the question this form is here to let somebody overrule.
  const chosen = catalogue.data?.chosen?.live ?? "";
  const model = speech.find((m) => m.id === chosen);
  const [mode, setMode] = useState<"language" | "model">(chosen ? "model" : "language");

  /** What the daemon would pick for a language, so an override can be named as one. */
  const bestFor = useCallback(
    (code: string) => languages.data?.languages.find((l) => l.code === code),
    [languages.data],
  );

  /** What the ranking would use for the chosen language, named so the mode can be checked. */
  const serving = mode === "language" ? bestFor(current) : undefined;

  /**
   * Languages whose recommendation the user has waved away, for this visit.
   *
   * Not persisted. A dismissal that outlives the session would hide the advice from somebody who
   * has since installed something or changed their mind, and the whole point of the panel is that
   * it reflects what is true now.
   */
  const [declined, setDeclined] = useState<string[]>([]);
  const better = declined.includes(current) ? undefined : betterFor(serving);

  /** Install the recommended model and point the `live` role at it — both, or neither. */
  const adopt = async (id: string) => {
    setBusy(true);
    setError(null);
    try {
      const started = await fetch(url(handshake, "/installs"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ id }),
      });
      if (!started.ok) throw new Error(await started.text());
      // Pinned now rather than after the download: the daemon honours the setting when the files
      // arrive, and a user who pressed "use this one" has said what they want regardless of how
      // long the bytes take.
      await write({ model: id });
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const write = async (next: { model?: string; language?: string }) => {
    setBusy(true);
    setError(null);
    try {
      if (next.model !== undefined) {
        const response = await fetch(url(handshake, "/settings/models"), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ role: "live", model: next.model }),
        });
        if (!response.ok) throw new Error(await response.text());
      }
      if (next.language !== undefined) await rememberLanguage(handshake, next.language);
      setGeneration((n) => n + 1);
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  // An empty *registry* is not an error and not a form: it is the models screen's job, and the
  // recording banner already sends people there. Nothing *installed* no longer lands here — the
  // form lists what could be installed, which is the state a new user is actually in and the one
  // where a blank panel saying "install something first" is least useful.
  if (catalogue.data && speech.length === 0) {
    return (
      <section
        className="border-line bg-bg-raised mt-6 rounded-2xl border p-5"
        data-testid="settings-recognition"
      >
        <h3 className="font-medium">{t("settings.recognition_heading")}</h3>
        <p className="text-fg-dim text-meta mt-1">{t("settings.recognition_none")}</p>
      </section>
    );
  }

  const recommended = mode === "model" && model ? bestFor(current) : undefined;
  const overruled =
    recommended?.model && model && recommended.model !== model.id ? recommended : undefined;

  return (
    <section
      className="border-line bg-bg-raised mt-6 rounded-2xl border p-5"
      data-testid="settings-recognition"
    >
      <h3 className="font-medium">{t("settings.recognition_heading")}</h3>
      <p className="text-fg-dim text-meta mt-1 mb-3">{t("settings.recognition_hint")}</p>

      <SegmentedControl
        label={t("settings.recognition_heading")}
        size="sm"
        options={[
          { value: "language" as const, label: t("settings.by_language") },
          { value: "model" as const, label: t("settings.by_model") },
        ]}
        value={mode}
        onChange={(next) => {
          setMode(next);
          // Going back to "by language" means giving the ranking its job back, so the pinned model
          // is cleared rather than left behind to keep winning silently.
          if (next === "language") void write({ model: "" });
        }}
      />

      {/* The other half of the choice, which this form did not have.

          "By model" got two dropdowns and "by language" got nothing at all — the heading, the
          sentence and the switch, and then the panel ended. The language was set somewhere else
          entirely (the picker above the record button, and the first-run screen), so a user who
          came to Settings to choose a language found a section named after the decision, offering
          no way to make it. One dropdown, and a line naming what will serve the answer, which is
          the fact that makes "by language" a defensible default rather than a black box. */}
      {mode === "language" && (
        <div className="mt-4">
          <label className="block sm:max-w-xs">
            <span className="text-fg-faint text-meta">{t("record.spoken")}</span>
            <Select
              className="mt-1"
              value={current}
              aria-label={t("record.spoken")}
              disabled={busy}
              onChange={(e) => void write({ language: e.target.value })}
            >
              {/* Detection is only offered where an installed model can actually do it. Naming it
                  otherwise would promise a behaviour the machine cannot perform. */}
              {autoAvailable(languages.data?.languages ?? []) && (
                <option value="">{t("record.spoken_auto")}</option>
              )}
              {ordered(
                (languages.data?.languages ?? [])
                  .filter((l) => !l.multilingual_only)
                  .map((l) => l.code),
                locale,
              ).map((each) => (
                <option key={each.code} value={each.code}>
                  {each.label}
                </option>
              ))}
            </Select>
          </label>

          {/* What will actually hear you, which is not the same as what is recommended.
              This named the *recommended* model, so a machine with only Whisper was told
              "Gipformer sẽ nghe" — a model it does not have and was never going to use. */}
          {serving && (
            <p className="text-fg-faint text-micro mt-2">
              {serving.serving_name
                ? t("settings.served_by", { model: serving.serving_name })
                : t("settings.served_by_none")}
            </p>
          )}

          {/* A recommendation, offered rather than applied.
              The app must never swap a model on somebody's behalf — least of all to one that is
              not installed. So the gap is stated with both numbers and closed by a button, and
              declining it leaves everything exactly as it is. */}
          {better && (
            <div className="border-accent/30 bg-accent-soft text-meta mt-3 rounded-lg border px-3 py-2">
              <p className="text-fg-dim">
                {t("settings.better_available", {
                  model: better.model_name ?? better.model ?? "",
                  better: String(Math.round(better.accuracy * 100)),
                  current: String(Math.round(better.serving_accuracy * 100)),
                  language: languageName(current, locale),
                  size: size(better.size_bytes),
                })}
              </p>
              <div className="mt-2 flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  busy={busy}
                  onClick={() => void adopt(better.model as string)}
                >
                  {t("settings.install_and_use")}
                </Button>
                <button
                  type="button"
                  onClick={() => setDeclined((was) => [...was, better.code])}
                  className="text-fg-faint text-micro underline"
                >
                  {t("settings.keep_current")}
                </button>
              </div>
            </div>
          )}
        </div>
      )}

      {mode === "model" && (
        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <label className="block">
            <span className="text-fg-faint text-meta">{t("settings.the_model")}</span>
            <Select
              className="mt-1"
              value={model?.id ?? ""}
              aria-label={t("settings.the_model")}
              disabled={busy}
              onChange={(e) => void write({ model: e.target.value })}
            >
              <option value="">{t("settings.pick_a_model")}</option>
              {speech.map((each) => (
                <option key={each.id} value={each.id}>
                  {/* The size on the ones that are not here yet, so the list is a list of choices
                      with prices rather than a list that quietly omits the expensive ones. */}
                  {each.installed ? each.name : `${each.name} · ${size(each.size_bytes)}`}
                </option>
              ))}
            </Select>
          </label>

          <label className="block">
            <span className="text-fg-faint text-meta">{t("settings.the_language")}</span>
            <Select
              className="mt-1"
              value={current}
              aria-label={t("settings.the_language")}
              // Every language *this* model claims, which for a multilingual one is the ninety-nine
              // the card lists. Ordered by `ordered`, which puts the reader's own language first:
              // sorted by name, "Tiếng Việt" is eightieth of ninety-nine, and a Vietnamese speaker
              // who scrolled a screen and a half concluded Whisper does not support Vietnamese.
              disabled={busy || !model}
              onChange={(e) => void write({ language: e.target.value })}
            >
              <option value="">{t("settings.detect_it")}</option>
              {ordered(model?.langs ?? [], locale).map((each) => (
                <option key={each.code} value={each.code}>
                  {each.label}
                </option>
              ))}
            </Select>
          </label>
        </div>
      )}

      {/* Chosen but not on disk. Not an error — it is a decision the user has made and a download
          they have not — so it reads as the next step rather than as a rejection. Recording with
          this pinned would fail at the moment of pressing record, which is the wrong place to find
          out. */}
      {mode === "model" && model && !model.installed && (
        <p className="border-accent/30 bg-accent-soft text-meta mt-3 flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2">
          <span className="text-fg-dim">
            {t("settings.model_not_installed", {
              model: model.name,
              size: size(model.size_bytes),
            })}
          </span>
          <button
            type="button"
            onClick={() => void navigate({ to: "/models" })}
            className="text-accent font-medium underline"
          >
            {t("record.manage_models")}
          </button>
        </p>
      )}

      {/* Overruling the ranking is allowed and is worth saying out loud, once, without a number
          nobody measured for this pair. */}
      {overruled && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta mt-3 rounded-lg border px-3 py-2">
          {t("settings.model_overruled", {
            language: languageName(current, locale),
            model: overruled.model_name ?? overruled.model ?? "",
          })}
        </p>
      )}

      {error && (
        <p role="alert" className="text-danger text-meta mt-3">
          {error}
        </p>
      )}
    </section>
  );
}
