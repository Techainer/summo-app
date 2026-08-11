import { useCallback, useEffect, useState } from "react";

import { useI18n, useT } from "../i18n/context";
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

// Product names stay as they are — "Ollama" is "Ollama" in every language. The labels and hints
// that are prose carry keys and resolve at render.
const PROVIDERS = [
  { value: "ollama", label: "Ollama", hintKey: "settings.local_only" },
  { value: "lm-studio", label: "LM Studio", hintKey: "settings.local_only" },
  { value: "openai", label: "OpenAI", hintKey: "settings.key_needed" },
  { value: "anthropic", label: "Anthropic", hintKey: "settings.key_needed" },
  { value: "custom", labelKey: "settings.other_endpoint", hintKey: "settings.endpoint_hint" },
] as { value: string; label?: string; labelKey?: string; hintKey: string }[];

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

  if (!llm) return <p className="empty">{status ?? t("settings.loading")}</p>;

  // The stored provider is either a known name or a URL; the picker shows t("settings.other_endpoint") for a URL.
  const selected = PROVIDERS.some((p) => p.value === llm.provider) ? llm.provider : "custom";
  const chosen = PROVIDERS.find((p) => p.value === selected);
  const needsKey = selected === "openai" || selected === "anthropic";

  return (
    <div className="settings" data-testid="settings">
      <LanguagePicker />

      <h2>{t("settings.llm_heading")}</h2>
      <p className="hint">
        Nhận dạng giọng nói và tách người nói <b>{t("settings.always_local")}</b>. Chỉ tóm tắt, dịch và hỏi
        đáp mới dùng mô hình ngôn ngữ — và bạn chọn nó chạy ở đâu.
      </p>

      <label className="field">
        <span>{t("settings.provider")}</span>
        <select
          value={selected}
          aria-label={t("settings.provider")}
          onChange={(e) => {
            const value = e.target.value;
            void save({ ...llm, provider: value === "custom" ? custom || "http://" : value });
          }}
        >
          {PROVIDERS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label ?? t(p.labelKey ?? "")}
            </option>
          ))}
        </select>
      </label>
      {chosen && <p className="field-hint">{t(chosen.hintKey)}</p>}

      {selected === "custom" && (
        <label className="field">
          <span>{t("settings.address")}</span>
          <input
            value={custom}
            aria-label={t("settings.endpoint")}
            placeholder="http://127.0.0.1:1234/v1"
            onChange={(e) => setCustom(e.target.value)}
            onBlur={() => custom.startsWith("http") && void save({ ...llm, provider: custom })}
          />
        </label>
      )}

      <label className="field">
        <span>{t("settings.model")}</span>
        <input
          value={llm.model ?? ""}
          aria-label={t("settings.model")}
          placeholder="qwen3:8b"
          onChange={(e) => setLlm({ ...llm, model: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className="field">
        <span>{t("settings.summary_language")}</span>
        <input
          value={llm.language}
          aria-label={t("settings.summary_language")}
          onChange={(e) => setLlm({ ...llm, language: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className="field checkbox">
        <input
          type="checkbox"
          checked={llm.summarize_on_stop}
          onChange={(e) => void save({ ...llm, summarize_on_stop: e.target.checked })}
        />
        <span>{t("settings.summarise_on_stop")}</span>
      </label>

      {needsKey && (
        <p className={`field-hint${keyPresent ? "" : " warn"}`}>
          {keyPresent
            ? t("settings.key_present")
            : t("settings.key_missing")}
          {" "}
          Khoá không được lưu vào tệp cài đặt — nếu lưu, nó sẽ theo vào bản sao lưu và bản đồng bộ.
        </p>
      )}

      <div className="settings-actions">
        <button type="button" className="primary" onClick={() => void test()} disabled={testing}>
          {testing ? t("settings.testing") : t("settings.test")}
        </button>
        {status && <span className="field-hint">{status}</span>}
      </div>

      {result && (
        <p className={`banner ${result.ok ? "ok" : "error"}`} data-testid="test-result">
          {result.ok ? t("settings.connected") : t("settings.not_connected")} — {result.base_url}
          <br />
          {result.local
            ? t("settings.local_only_comma")
            : t("settings.sent_here")}
          <br />
          <span className="field-hint">{result.detail}</span>
        </p>
      )}
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
      <h2>{t("settings.language")}</h2>
      <label className="field">
        <span>{t("settings.language")}</span>
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
      <p className="hint">{t("settings.language_hint")}</p>
    </>
  );
}
