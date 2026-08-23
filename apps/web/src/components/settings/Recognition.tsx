import { useCallback, useMemo, useState } from "react";
import { useNavigate } from "@tanstack/react-router";

import { useI18n } from "../../i18n/context";
import { CatalogueClient, size } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import {
  autoAvailable,
  fetchLanguages,
  languageName,
  ordered,
  rememberLanguage,
} from "../../lib/languages";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { SegmentedControl, Select } from "../ui";

/**
 * Which model does the listening, and in which language.
 *
 * The app has always answered this the other way round. You pick a *language*, it ranks the
 * installed models for it and uses the winner — which is the right default and is not a choice.
 * Vietnamese always resolves to Gipformer, because on Vietnamese Gipformer measures 91.5 % and
 * Whisper 34.5 %, so Whisper is never offered for it. Somebody who wants Whisper anyway — for a
 * meeting that will switch languages, or to compare the two — had no way to say so.
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
 * ## It says when the choice is worse
 *
 * The point of letting somebody overrule the ranking is that they sometimes should. The point of
 * measuring models is that usually they should not. So when the chosen model is not the one the
 * daemon would have picked for that language, this names the one it would have — without inventing
 * a number for the pair, because `/languages` reports accuracy for the *best* model per language
 * and not for whichever one is being overruled. Saying "Gipformer is recommended here" is
 * supportable; printing a percentage for Whisper-on-Vietnamese from that endpoint would not be.
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

          {/* Which model the ranking picked, and whether it is here. Without this the mode is a
              promise that something sensible happens and no way to check that it did. */}
          {serving && (
            <p className="text-fg-faint text-micro mt-2">
              {serving.installed && serving.model_name
                ? t("settings.served_by", { model: serving.model_name })
                : t("settings.served_by_missing", {
                    model: serving.model_name ?? serving.model ?? "",
                    size: size(serving.size_bytes),
                  })}
            </p>
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
