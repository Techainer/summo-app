import { useCallback, useEffect, useState } from "react";

import { useI18n, useT } from "../i18n/context";
import { About } from "./About";
import { url } from "../lib/library";
import type { Handshake } from "../lib/engine";

/**
 * Settings, which is only ever the language model.
 *
 * Everything else about Summo is either measured (which ASR model fits this machine) or a
 * consequence of a file living on disk. The language model is the one thing the app genuinely
 * cannot decide for the user, because it is the only part of the product that can send their words
 * somewhere else — so it is the only thing this screen asks about.
 */

/**
 * One endpoint, as the daemon describes it.
 *
 * This list used to live here as a hardcoded array of four, alongside the daemon's own list of
 * four. Adding a provider meant editing both, in two languages, and two facts could not be shown at
 * all because only the daemon had them: which environment variable holds the key, and whether it is
 * already set on this machine.
 */
interface ProviderInfo {
  id: string;
  name: string;
  base_url: string;
  model: string;
  local: boolean;
  key_env: string | null;
  key_set: boolean;
}

/** One row of the form: a fixed-width label beside its control. */
const FIELD = "mt-3.5 flex items-center gap-3 text-[13px] text-fg-dim";
const LABEL = "w-[150px] shrink-0";
const CONTROL =
  "flex-1 rounded-lg border border-line bg-bg-soft px-2.5 py-1.5 text-sm text-fg focus:outline-none focus-visible:border-accent";
/** The note under a field. Indented to line up with the control it explains. */
const HINT = "mt-1.5 ml-[162px] text-[12px] leading-normal text-fg-faint";

/** The pseudo-entry for "some other OpenAI-compatible server". Not a preset; there is no list of them. */
const CUSTOM = "custom";

interface Llm {
  provider: string;
  model: string | null;
  language: string;
  summarize_on_stop: boolean;
}

interface TestResult {
  ok: boolean;
  base_url: string;
  local: boolean;
  detail: string;
}

export function Settings({ handshake }: { handshake: Handshake }) {
  const t = useT();
  const [llm, setLlm] = useState<Llm | null>(null);
  const [providers, setProviders] = useState<ProviderInfo[]>([]);
  const [keyPresent, setKeyPresent] = useState(false);
  const [custom, setCustom] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [result, setResult] = useState<TestResult | null>(null);
  const [testing, setTesting] = useState(false);

  useEffect(() => {
    fetch(url(handshake, "/settings"))
      .then((r) => r.json())
      .then((body: { settings: { llm: Llm }; api_key_present: boolean }) => {
        setLlm(body.settings.llm);
        setKeyPresent(body.api_key_present);
        if (body.settings.llm.provider.startsWith("http")) setCustom(body.settings.llm.provider);
      })
      .catch((e: unknown) => setStatus(e instanceof Error ? e.message : String(e)));

    fetch(url(handshake, "/settings/llm/providers"))
      .then((r) => r.json())
      .then((body: { providers: ProviderInfo[] }) => setProviders(body.providers))
      .catch((e: unknown) => setStatus(e instanceof Error ? e.message : String(e)));
  }, [handshake]);

  const post = useCallback(
    async (path: string, body: Llm) => {
      const response = await fetch(url(handshake, path), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body),
      });
      const parsed = (await response.json()) as Record<string, unknown>;
      if (!response.ok) throw new Error(String(parsed.error ?? response.statusText));
      return parsed;
    },
    [handshake],
  );

  const save = useCallback(
    async (next: Llm) => {
      setLlm(next);
      setResult(null);
      try {
        await post("/settings/llm", next);
        setStatus(t("settings.saved"));
      } catch (e) {
        setStatus(e instanceof Error ? e.message : String(e));
      }
    },
    [post],
  );

  const test = useCallback(async () => {
    if (!llm) return;
    setTesting(true);
    setResult(null);
    try {
      setResult((await post("/settings/llm/test", llm)) as unknown as TestResult);
    } catch (e) {
      setStatus(e instanceof Error ? e.message : String(e));
    } finally {
      setTesting(false);
    }
  }, [llm, post]);

  if (!llm) return <p className="mt-16 text-center text-fg-faint">{status ?? t("settings.loading")}</p>;

  // The stored provider is either a known id or a URL; the picker shows t("settings.other_endpoint") for a URL.
  const selected = providers.some((p) => p.id === llm.provider) ? llm.provider : CUSTOM;
  const chosen = providers.find((p) => p.id === selected);
  // Whether a key is wanted is the daemon's answer, not a list of names kept in sync by hand.
  const needsKey = chosen?.key_env != null;
  // Local endpoints are grouped first: it is the recommendation, and the difference between them
  // and the rest is the entire product promise.
  const local = providers.filter((p) => p.local);
  const hosted = providers.filter((p) => !p.local);

  return (
    <div className="max-w-xl p-6" data-testid="settings">
      <LanguagePicker />

      <h2 className="text-xl font-semibold tracking-tight">{t("settings.llm_heading")}</h2>
      <p className="my-4 text-[13px] leading-normal text-fg-faint">
        {t("settings.llm_hint_head")}
        <b>{t("settings.always_local")}</b>
        {t("settings.llm_hint_tail")}
      </p>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.provider")}</span>
        <select
          className={CONTROL}
          value={selected}
          aria-label={t("settings.provider")}
          onChange={(e) => {
            const value = e.target.value;
            void save({ ...llm, provider: value === "custom" ? custom || "http://" : value });
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
        </select>
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

      {selected === "custom" && (
        <label className={FIELD}>
          <span className={LABEL}>{t("settings.address")}</span>
          <input
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
        <input
          className={CONTROL}
          value={llm.model ?? ""}
          aria-label={t("settings.model")}
          placeholder="qwen3:8b"
          onChange={(e) => setLlm({ ...llm, model: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.summary_language")}</span>
        <input
          className={CONTROL}
          value={llm.language}
          aria-label={t("settings.summary_language")}
          onChange={(e) => setLlm({ ...llm, language: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className="mt-3.5 flex items-center gap-2.5 text-[13px] text-fg-dim">
        <input
          type="checkbox"
          checked={llm.summarize_on_stop}
          onChange={(e) => void save({ ...llm, summarize_on_stop: e.target.checked })}
        />
        <span>{t("settings.summarise_on_stop")}</span>
      </label>

      {needsKey && (
        <p className={`${HINT} ${keyPresent ? "" : "text-blocked"}`}>
          {keyPresent
            ? t("settings.key_present")
            : t("settings.key_missing")}
          {" "}
          {t("settings.key_not_stored")}
        </p>
      )}

      <div className="mt-5 flex flex-wrap items-center gap-3">
        <button
          type="button"
          onClick={() => void test()}
          disabled={testing}
          className="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition-opacity hover:opacity-90 disabled:opacity-60"
        >
          {testing ? t("settings.testing") : t("settings.test")}
        </button>
        {status && <span className="text-[12px] text-fg-faint">{status}</span>}
      </div>

      {result && (
        <p
          data-testid="test-result"
          className={`mt-3 flex flex-wrap items-center gap-2 rounded-lg border px-3 py-2 text-[13px] ${
            result.ok
              ? "border-accent/30 bg-accent-soft"
              : "border-rec/30 bg-rec-soft text-rec"
          }`}
        >
          {result.ok ? t("settings.connected") : t("settings.not_connected")} — {result.base_url}
          <br />
          {result.local
            ? t("settings.local_only_comma")
            : t("settings.sent_here")}
          <br />
          <span className="text-[12px] text-fg-faint">{result.detail}</span>
        </p>
      )}
      <About />
    </div>
  );
}

/**
 * Which language the interface is in.
 *
 * Listed by each language's own name — someone looking for their language cannot read a list
 * written in a language they do not read, which is why "Tiếng Việt" is not "Vietnamese".
 *
 * The hint about `~/.summo/locales/` is the whole contribution process, so it belongs on screen
 * rather than in a document nobody opens.
 */
function LanguagePicker() {
  const { locale, setLocale, languages, t } = useI18n();

  return (
    <>
      <h2 className="text-xl font-semibold tracking-tight">{t("settings.language")}</h2>
      <label className={FIELD}>
        <span className={LABEL}>{t("settings.language")}</span>
        <select
          value={locale}
          aria-label={t("settings.language")}
          onChange={(e) => setLocale(e.target.value)}
        >
          {languages.map((language) => (
            <option key={language.code} value={language.code}>
              {language.label}
            </option>
          ))}
        </select>
      </label>
      <p className="my-4 text-[13px] leading-normal text-fg-faint">{t("settings.language_hint")}</p>
    </>
  );
}
