import { useNavigate } from "@tanstack/react-router";
import { useCallback, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import {
  AUTO,
  autoAvailable,
  fetchLanguages,
  languageName,
  megabytes,
  quality,
  ready,
  rememberLanguage,
  type Language,
} from "../../lib/languages";
import { OnboardingClient, percent } from "../../lib/onboarding";
import { useLoad } from "../../lib/use-load";
import { Button } from "../ui";

/**
 * Which language is being spoken, and the model that can hear it.
 *
 * The question this asks used to be answered by inference — setup recommended a model for whatever
 * language the *interface* was in. Right often enough that being wrong was invisible, and being
 * wrong meant a download that could not transcribe the meeting it was installed for.
 *
 * What makes this more than a `<select>`:
 *
 * **Every option says what it costs and what it is worth.** A language served by a model that is
 * not installed shows the download size; one served only through a multilingual model's `*` says
 * it was never measured on this language. Whisper covers Vietnamese at 34 % and a 73 MB transducer
 * does it at 91 %, and a picker that hides that is recommending the wrong one.
 *
 * **A missing model is a download, not a refusal.** Choosing a language nothing is installed for
 * offers the download right here, with progress, and the record button waits rather than failing.
 * The alternative — the state this replaces — is a record button that stops with "no model" and a
 * user who has to work out which of eight to install.
 *
 * **Automatic is offered only when it can work.** Detection is Whisper's own, so it needs a
 * multilingual model on disk; it also costs accuracy and can flip mid-meeting, so it is never the
 * default.
 */
/**
 * The value of the "several languages" entry.
 *
 * Not a language code, and not the empty string it resolves to: a `<select>` needs a value that is
 * distinct from every option beside it, and the empty string is already taken by "as configured".
 */
const MULTI = "__multi__";

export function SpokenLanguage({
  value,
  onChange,
  compact = false,
}: {
  /** Language code, or `AUTO` for detection. */
  value: string;
  onChange: (code: string) => void;
  /** Drop the explanation line — for the record bar, where space is a row. */
  compact?: boolean;
}) {
  const navigate = useNavigate();
  const { handshake } = useEngine();
  const { t, locale } = useI18n();
  const [installing, setInstalling] = useState<{ id: string; pct: number } | null>(null);

  const probe = useLoad(
    useCallback(async () => fetchLanguages(handshake), [handshake]),
    [handshake],
  );
  const languages = probe.data?.languages ?? [];
  const chosen = languages.find((language) => language.code === value);

  // The download, watched until it finishes. `OnboardingClient` already knows how to start one and
  // how to report it; this only has to keep asking, because the alternative is a spinner that never
  // ends when a download fails.
  const install = async (language: Language) => {
    if (!language.model) return;
    const client = new OnboardingClient(handshake);
    setInstalling({ id: language.model, pct: 0 });
    try {
      await client.install(language.model);
      for (;;) {
        const jobs = await client.installs();
        const job = jobs.find((candidate) => candidate.model === language.model);
        if (!job || job.state === "done") break;
        if (job.state === "failed") throw new Error(job.error ?? "install failed");
        // `percent` is null while a job is queued or has no content length; the last known number
        // is better than a bar that jumps back to zero every time the answer is unknown.
        setInstalling((current) => ({
          id: language.model ?? "",
          pct: percent(job) ?? current?.pct ?? 0,
        }));
        await new Promise((resolve) => setTimeout(resolve, 700));
      }
      probe.reload();
    } finally {
      setInstalling(null);
    }
  };

  // Measured languages first, then the rest by their *displayed* name. The daemon sorts by code,
  // which is right for a wire format and wrong for a list a person scrolls: in Vietnamese, `af`
  // renders as "Tiếng Hà Lan (Nam Phi)" and sits between English and Vietnamese for no reason a
  // reader can see.
  // The multilingual entry: one model that covers everything, detecting per utterance. Offered
  // whether or not it is installed, because "this meeting is in two languages" is a thing a user
  // knows before they own a model for it — and choosing it should start the download, exactly as
  // choosing a language does.
  const multilingual = languages.find((language) => language.multilingual_only && language.model);

  const options = languages
    .filter((language) => language.model)
    .sort((a, b) => {
      if (a.accuracy !== b.accuracy) return b.accuracy - a.accuracy;
      return languageName(a.code, locale).localeCompare(languageName(b.code, locale), locale);
    });
  const auto = autoAvailable(languages);

  return (
    <div className={compact ? "flex flex-wrap items-center gap-2" : ""}>
      <label className="text-fg-faint flex items-center gap-2 text-sm">
        {t("record.spoken")}
        <select
          value={value}
          aria-label={t("record.spoken")}
          onChange={(event) => {
            const raw = event.target.value;
            // `MULTI` is not a language, it is a request for one model and no language. It resolves
            // to the same empty code the daemon and sherpa-onnx already mean by "detect".
            const code = raw === MULTI ? AUTO : raw;
            if (raw === MULTI && multilingual && !multilingual.installed)
              void install(multilingual);
            onChange(code);
            // Written through to the daemon so the choice survives this browser. A failure here is
            // deliberately swallowed: the recording still has the language, and a preference that
            // could not be saved must not stop it.
            void rememberLanguage(handshake, code).catch(() => undefined);
          }}
          className="border-line bg-bg-soft text-fg hover:border-line-strong focus-visible:border-accent h-8 max-w-56 rounded-[var(--radius-card)] border px-2 text-sm transition-colors focus:outline-none"
        >
          {/* Detection first when it is possible, because somebody who does not know what will be
              spoken is exactly who needs it. */}
          {/* First, always, whichever it is: the option that describes what the control is doing
              *now*. With detection available that is "automatic"; without it, "as configured" —
              and without either the browser would show the first language in the list, so a
              control meaning "whatever the settings say" silently claimed to be recording English,
              the one wrong answer that never announces itself. */}
          {auto ? (
            <option value={AUTO}>{t("record.spoken_auto")}</option>
          ) : (
            value === AUTO && <option value={AUTO}>{t("record.spoken_default")}</option>
          )}

          {/* Then the multilingual entry: one model that hears everything, detecting per utterance.
              Offered whether or not it is installed, because "this meeting is in two languages" is
              something a user knows before they own a model for it. */}
          {!auto && multilingual && (
            <option value={MULTI}>
              {t("record.spoken_multi")}
              {multilingual.installed ? "" : ` · ${megabytes(multilingual.size_bytes)}`}
            </option>
          )}
          {options.map((language) => (
            <option key={language.code} value={language.code}>
              {languageName(language.code, locale)}
              {language.installed ? "" : ` · ${megabytes(language.size_bytes)}`}
            </option>
          ))}
        </select>
      </label>
      {/* The way out of a list that does not have what somebody wants: the catalogue, filtered to
          the language they just chose. Every model that serves it, with its size, its measured
          accuracy and its page — rather than a picker that can only offer what it already knows. */}
      <button
        type="button"
        onClick={() =>
          void navigate({
            to: "/models",
            search: value && value !== AUTO ? { lang: value } : {},
          })
        }
        className="text-fg-faint hover:text-fg text-micro underline"
      >
        {t("record.browse_models")}
      </button>

      {/* The model that will actually run, named. Two languages can resolve to the same model and
          one language can resolve to a model the user did not expect; saying which removes both
          surprises. */}
      {!compact && chosen?.model_name && (
        <p className="text-fg-dim text-meta mt-2">
          {t("record.spoken_model", { model: chosen.model_name })}
          {quality(chosen) === "unmeasured" && ` — ${t("record.spoken_unmeasured")}`}
          {quality(chosen) === "poor" && ` — ${t("record.spoken_poor")}`}
          {!chosen.live && ` — ${t("record.spoken_slow")}`}
        </p>
      )}

      {chosen && !ready(chosen) && (
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <Button
            onClick={() => void install(chosen)}
            disabled={installing !== null}
            variant={compact ? "ghost" : "primary"}
          >
            {installing
              ? t("record.spoken_installing", { pct: String(installing.pct) })
              : t("record.spoken_install", { size: megabytes(chosen.size_bytes) })}
          </Button>
          {!compact && <span className="text-fg-faint text-micro">{t("record.spoken_wait")}</span>}
        </div>
      )}

      {probe.error && <p className="text-rec text-micro mt-2">{probe.error}</p>}
    </div>
  );
}
