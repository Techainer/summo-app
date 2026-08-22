import { useCallback, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { CatalogueClient, type CatalogueModel } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { fetchLanguages, languageName, rememberLanguage } from "../../lib/languages";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { SegmentedControl } from "../ui";

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
  const { handshake } = useEngine();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** Bumped after a write, so both lists are re-read rather than assumed. */
  const [generation, setGeneration] = useState(0);

  const catalogue = useLoad(
    useCallback(
      async () => new CatalogueClient(handshake).list(),
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

  /** Only what is on this machine: a model that is not installed cannot do the listening. */
  const installed = useMemo(
    () =>
      (catalogue.data?.models ?? []).filter(
        (model) => model.installed && model.task === "asr" && model.langs.length > 0,
      ),
    [catalogue.data],
  );

  const current = languages.data?.current ?? "";
  const chosen = languages.data?.model ?? "";
  const model = installed.find((m) => m.id === chosen);
  const [mode, setMode] = useState<"language" | "model">(chosen ? "model" : "language");

  /** What the daemon would pick for a language, so an override can be named as one. */
  const bestFor = useCallback(
    (code: string) => languages.data?.languages.find((l) => l.code === code),
    [languages.data],
  );

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

  // Nothing installed is not an error and not a form: it is the models screen's job, and the
  // recording banner already sends people there.
  if (catalogue.data && installed.length === 0) {
    return (
      <section className="border-line bg-bg-raised mt-6 rounded-2xl border p-5">
        <h3 className="font-medium">{t("settings.recognition_heading")}</h3>
        <p className="text-fg-dim text-meta mt-1">{t("settings.recognition_none")}</p>
      </section>
    );
  }

  const recommended = mode === "model" && model ? bestFor(current) : undefined;
  const overruled =
    recommended?.model && model && recommended.model !== model.id ? recommended : undefined;

  return (
    <section className="border-line bg-bg-raised mt-6 rounded-2xl border p-5" data-testid="settings-recognition">
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

      {mode === "model" && (
        <div className="mt-4 grid gap-3 sm:grid-cols-2">
          <label className="block">
            <span className="text-fg-faint text-meta">{t("settings.the_model")}</span>
            <select
              value={model?.id ?? ""}
              aria-label={t("settings.the_model")}
              disabled={busy}
              onChange={(e) => void write({ model: e.target.value })}
              className="border-line bg-bg-soft text-fg hover:border-line-strong focus-visible:border-accent mt-1 h-9 w-full rounded-[var(--radius-card)] border px-2 text-sm transition-colors focus:outline-none"
            >
              <option value="">{t("settings.pick_a_model")}</option>
              {installed.map((each) => (
                <option key={each.id} value={each.id}>
                  {each.name}
                </option>
              ))}
            </select>
          </label>

          <label className="block">
            <span className="text-fg-faint text-meta">{t("settings.the_language")}</span>
            <select
              value={current}
              aria-label={t("settings.the_language")}
              // Every language *this* model claims, which for a multilingual one is the ninety-nine
              // the card now lists. Sorted by name in the reader's own language: a list ordered by
              // ISO code puts Vietnamese between Urdu and Yiddish.
              disabled={busy || !model}
              onChange={(e) => void write({ language: e.target.value })}
              className="border-line bg-bg-soft text-fg hover:border-line-strong focus-visible:border-accent mt-1 h-9 w-full rounded-[var(--radius-card)] border px-2 text-sm transition-colors focus:outline-none disabled:opacity-[var(--disabled-opacity)]"
            >
              <option value="">{t("settings.detect_it")}</option>
              {[...(model?.langs ?? [])]
                .map((code) => ({ code, label: languageName(code, locale) }))
                .sort((a, b) => a.label.localeCompare(b.label, locale))
                .map((each) => (
                  <option key={each.code} value={each.code}>
                    {each.label}
                  </option>
                ))}
            </select>
          </label>
        </div>
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
