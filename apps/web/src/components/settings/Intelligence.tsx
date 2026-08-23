import { Checkbox, Input, Select } from "../ui";
import { CONTROL, FIELD, HINT, LABEL } from "./fields";
import { useI18n, useT } from "../../i18n/context";
import { languageName } from "../../lib/languages";
import { CUSTOM, type LlmSettings } from "./llm";

/**
 * The languages a summary can be written in, offered as a list rather than typed.
 *
 * The stored value is the English name, because it goes into a prompt — "write this in Vietnamese"
 * is what a model understands, and a field holding `Tiếng Việt` would have quietly changed what the
 * model was asked for. So the value is English and the label is not: a Vietnamese interface asking
 * somebody to type the word "Vietnamese" was the interface leaking its own plumbing.
 */
const SUMMARY_LANGUAGES: { value: string; code: string }[] = [
  { value: "Vietnamese", code: "vi" },
  { value: "English", code: "en" },
  { value: "Japanese", code: "ja" },
  { value: "Chinese", code: "zh" },
  { value: "Korean", code: "ko" },
  { value: "French", code: "fr" },
  { value: "German", code: "de" },
  { value: "Spanish", code: "es" },
];

/**
 * Which model writes the summaries, and where it runs.
 *
 * The one thing about Summo the app genuinely cannot decide for the user: everything else is either
 * measured (which recognition model fits this machine) or a consequence of a file on disk, while
 * this is the only part of the product that can send their words somewhere else.
 */
export function Intelligence({ settings }: { settings: LlmSettings }) {
  const t = useT();
  const { locale } = useI18n();
  const {
    llm,
    providers,
    keyPresent,
    custom,
    setCustom,
    edit,
    save,
    test,
    testing,
    result,
    status,
  } = settings;
  if (!llm) return null;

  // The stored provider is either a known id or a URL; the picker shows "other" for a URL.
  const selected = providers.some((p) => p.id === llm.provider) ? llm.provider : CUSTOM;
  const chosen = providers.find((p) => p.id === selected);
  // Whether a key is wanted is the daemon's answer, not a list of names kept in step by hand.
  const needsKey = chosen?.key_env != null;
  // Local endpoints are grouped first: it is the recommendation, and the difference between them
  // and the rest is the entire product promise.
  const local = providers.filter((p) => p.local);
  const hosted = providers.filter((p) => !p.local);

  return (
    <div data-testid="settings-ai">
      <p className="text-fg-faint text-meta mb-4 leading-normal">
        {t("settings.llm_hint_head")}
        <b>{t("settings.always_local")}</b>
        {t("settings.llm_hint_tail")}
      </p>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.provider")}</span>
        <Select
          className={CONTROL}
          value={selected}
          aria-label={t("settings.provider")}
          onChange={(e) => {
            const value = e.target.value;
            void save({ ...llm, provider: value === CUSTOM ? custom || "http://" : value });
          }}
        >
          <optgroup label={t("settings.group_local")}>
            {local.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </optgroup>
          <optgroup label={t("settings.group_hosted")}>
            {hosted.map((p) => (
              // Ready means the key is already in the environment, so the user knows before they
              // press Test whether there is anything left to do.
              <option key={p.id} value={p.id}>
                {p.name}
                {p.key_set ? ` — ${t("settings.key_ready")}` : ""}
              </option>
            ))}
          </optgroup>
          <option value={CUSTOM}>{t("settings.other_endpoint")}</option>
        </Select>
      </label>
      <p className={HINT}>
        {selected === CUSTOM
          ? t("settings.endpoint_hint")
          : chosen?.local
            ? t("settings.local_only")
            : chosen?.key_set
              ? t("settings.key_found", { name: chosen.key_env ?? "" })
              : t("settings.key_missing", { name: chosen?.key_env ?? "" })}
      </p>

      {selected === CUSTOM && (
        <label className={FIELD}>
          <span className={LABEL}>{t("settings.address")}</span>
          <Input
            className={CONTROL}
            value={custom}
            aria-label={t("settings.endpoint")}
            placeholder="http://127.0.0.1:1234/v1"
            onChange={(e) => setCustom(e.target.value)}
            onBlur={() => custom.startsWith("http") && void save({ ...llm, provider: custom })}
          />
        </label>
      )}

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.model")}</span>
        <Input
          className={CONTROL}
          value={llm.model ?? ""}
          aria-label={t("settings.model")}
          placeholder="qwen3:8b"
          onChange={(e) => edit({ ...llm, model: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.summary_language")}</span>
        <Select
          className={CONTROL}
          value={llm.language}
          aria-label={t("settings.summary_language")}
          onChange={(e) => void save({ ...llm, language: e.target.value })}
        >
          {/* Whatever is stored, even if it is not on the list — a value typed before this was a
              list, or set by hand in the settings file, must not silently become Vietnamese. */}
          {!SUMMARY_LANGUAGES.some((one) => one.value === llm.language) && (
            <option value={llm.language}>{llm.language}</option>
          )}
          {SUMMARY_LANGUAGES.map(({ value, code }) => (
            <option key={value} value={value}>
              {languageName(code, locale)}
            </option>
          ))}
        </Select>
      </label>

      <Checkbox
        className="mt-3.5"
        checked={llm.summarize_on_stop}
        onChange={(summarize_on_stop) => void save({ ...llm, summarize_on_stop })}
      >
        {t("settings.summarise_on_stop")}
      </Checkbox>

      {needsKey && (
        <p className={`${HINT} ${keyPresent ? "" : "text-blocked"}`}>
          {keyPresent ? t("settings.key_present") : t("settings.key_missing")}{" "}
          {t("settings.key_not_stored")}
        </p>
      )}

      <div className="mt-5 flex flex-wrap items-center gap-3">
        <button
          type="button"
          onClick={() => void test()}
          disabled={testing}
          className="bg-accent text-accent-fg rounded-lg px-4 py-2 text-sm font-semibold transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {testing ? t("settings.testing") : t("settings.test")}
        </button>
        {status && <span className="text-fg-faint text-micro">{status}</span>}
      </div>

      {result && (
        <p
          data-testid="test-result"
          className={`text-meta mt-3 flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2 ${
            result.ok ? "border-accent/30 bg-accent-soft" : "border-rec/30 bg-rec-soft text-rec"
          }`}
        >
          {result.ok ? t("settings.connected") : t("settings.not_connected")} — {result.base_url}
          <br />
          {result.local ? t("settings.local_only_comma") : t("settings.sent_here")}
          <br />
          <span className="text-fg-faint text-micro">{result.detail}</span>
        </p>
      )}
    </div>
  );
}
