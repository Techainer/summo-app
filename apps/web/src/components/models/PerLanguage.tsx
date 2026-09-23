import { useI18n, useT } from "../../i18n/context";
import type { CatalogueModel } from "../../lib/catalogue";
import type { Language } from "../../lib/languages";
import { languageName } from "../../lib/languages";
import { Select } from "../ui";

/**
 * Which model listens to which language.
 *
 * The models screen had one "use" button, writing one model for every meeting in every language,
 * while the language picker went on offering a hundred. Press it on a Vietnamese card and English
 * was then recorded by a model that declares `vi` — no error, a transcript that reads like
 * something. Reported as *"sao chọn 1 model dùng, tiếng Việt thì lại không chọn được tiếng Anh"*:
 * choosing the model took the language away.
 *
 * The daemon keeps a choice per language now, and falls back to the global one for every language
 * nobody has decided about. This is where that decision is made, because a card cannot express it:
 * a card is one model, and the question is which of several answers for one language.
 *
 * ## Which languages appear
 *
 * Not ninety-nine. A row is drawn only when an installed model **names a language explicitly** — a
 * multilingual model's `*` is not a statement that somebody cares about Japanese — or when a choice
 * already exists for it. On an ordinary machine that is one or two rows; on a machine with nothing
 * installed it is none, which is why this renders nothing rather than an empty box with a heading.
 *
 * ## What the options are
 *
 * Only installed models that cover the language. Offering one that does not is offering the exact
 * mistake this section exists to prevent, and the daemon refuses it anyway — a select that produces
 * an error is worse than a select that never offered the option.
 */
export function PerLanguage({
  languages,
  models,
  busy,
  onChoose,
}: {
  languages: Language[];
  models: CatalogueModel[];
  /** The language whose row is waiting on the daemon, so its control can be held. */
  busy: string | null;
  onChoose: (language: string, model: string) => void;
}) {
  const t = useT();
  const { locale } = useI18n();

  const speech = models.filter((model) => model.task === "asr" && model.installed);
  // A language a model spells out, rather than one it reaches through `*`. `langs` arrives from the
  // daemon with the star already expanded into a hundred codes, so a multilingual model is
  // recognised by the size of its list rather than by its contents — see the note on `/catalogue`
  // about why the expansion happens there.
  const base = (code: string) => code.toLowerCase().split("-")[0] ?? code;
  const multilingual = (model: CatalogueModel) => model.langs.length > 20;

  const named = new Set<string>();
  for (const model of speech) {
    if (multilingual(model)) continue;
    for (const code of model.langs) named.add(base(code));
  }
  for (const language of languages) if (language.chosen) named.add(language.code);

  const rows = [...named]
    .map((code) => ({
      code,
      language: languages.find((each) => each.code === code),
      options: speech.filter(
        (model) => multilingual(model) || model.langs.some((each) => base(each) === code),
      ),
    }))
    // One option is not a choice: a row offering a single model says what is already true, and this
    // section is for the machines where there is something to decide.
    .filter((row) => row.options.length > 1)
    .sort((a, b) => a.code.localeCompare(b.code));

  if (rows.length === 0) return null;

  return (
    <section
      data-testid="per-language"
      className="border-line rounded-[var(--radius-card)] border p-4"
    >
      <h2 className="text-meta font-medium">{t("models.per_language")}</h2>
      <p className="text-fg-dim text-micro mt-1">{t("models.per_language_hint")}</p>
      <ul className="mt-3 flex flex-col gap-2">
        {rows.map((row) => (
          <li key={row.code} className="flex items-center gap-3">
            <span className="text-meta w-32 shrink-0 truncate">
              {languageName(row.code, locale)}
            </span>
            <Select
              size="sm"
              aria-label={t("models.per_language_for", {
                language: languageName(row.code, locale),
              })}
              value={row.language?.chosen ?? ""}
              disabled={busy === row.code}
              onChange={(event) => onChoose(row.code, event.target.value)}
            >
              {/* The ranking's answer, named. "Automatic" on its own would hide the fact that
                  something is already listening to this language today. */}
              <option value="">
                {row.language?.serving_name
                  ? t("models.per_language_auto_named", { model: row.language.serving_name })
                  : t("models.per_language_auto")}
              </option>
              {row.options.map((model) => (
                <option key={model.id} value={model.id}>
                  {model.name}
                </option>
              ))}
            </Select>
          </li>
        ))}
      </ul>
    </section>
  );
}
