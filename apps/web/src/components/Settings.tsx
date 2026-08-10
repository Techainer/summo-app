import { useCallback, useEffect, useState } from "react";

import { useI18n } from "../i18n/context";
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

const PROVIDERS = [
  { value: "ollama", label: "Ollama", hint: "Chạy trên máy bạn. Không có gì rời khỏi máy." },
  { value: "lm-studio", label: "LM Studio", hint: "Chạy trên máy bạn. Không có gì rời khỏi máy." },
  { value: "openai", label: "OpenAI", hint: "Cần SUMMO_API_KEY. Văn bản bản ghi sẽ được gửi đi." },
  { value: "anthropic", label: "Anthropic", hint: "Cần SUMMO_API_KEY. Văn bản bản ghi sẽ được gửi đi." },
  { value: "custom", label: "Endpoint khác", hint: "Bất cứ API nào tương thích OpenAI." },
];

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
        setStatus("Đã lưu");
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

  if (!llm) return <p className="empty">{status ?? "Đang đọc cài đặt…"}</p>;

  // The stored provider is either a known name or a URL; the picker shows "Endpoint khác" for a URL.
  const selected = PROVIDERS.some((p) => p.value === llm.provider) ? llm.provider : "custom";
  const chosen = PROVIDERS.find((p) => p.value === selected);
  const needsKey = selected === "openai" || selected === "anthropic";

  return (
    <div className="settings" data-testid="settings">
      <LanguagePicker />

      <h2>Mô hình ngôn ngữ</h2>
      <p className="hint">
        Nhận dạng giọng nói và tách người nói <b>luôn chạy trên máy bạn</b>. Chỉ tóm tắt, dịch và hỏi
        đáp mới dùng mô hình ngôn ngữ — và bạn chọn nó chạy ở đâu.
      </p>

      <label className="field">
        <span>Nhà cung cấp</span>
        <select
          value={selected}
          aria-label="Nhà cung cấp"
          onChange={(e) => {
            const value = e.target.value;
            void save({ ...llm, provider: value === "custom" ? custom || "http://" : value });
          }}
        >
          {PROVIDERS.map((p) => (
            <option key={p.value} value={p.value}>
              {p.label}
            </option>
          ))}
        </select>
      </label>
      {chosen && <p className="field-hint">{chosen.hint}</p>}

      {selected === "custom" && (
        <label className="field">
          <span>Địa chỉ</span>
          <input
            value={custom}
            aria-label="Địa chỉ endpoint"
            placeholder="http://127.0.0.1:1234/v1"
            onChange={(e) => setCustom(e.target.value)}
            onBlur={() => custom.startsWith("http") && void save({ ...llm, provider: custom })}
          />
        </label>
      )}

      <label className="field">
        <span>Mô hình</span>
        <input
          value={llm.model ?? ""}
          aria-label="Mô hình"
          placeholder="qwen3:8b"
          onChange={(e) => setLlm({ ...llm, model: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className="field">
        <span>Ngôn ngữ tóm tắt</span>
        <input
          value={llm.language}
          aria-label="Ngôn ngữ tóm tắt"
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
        <span>Tự tóm tắt khi dừng ghi</span>
      </label>

      {needsKey && (
        <p className={`field-hint${keyPresent ? "" : " warn"}`}>
          {keyPresent
            ? "Đã có SUMMO_API_KEY."
            : "Chưa có SUMMO_API_KEY. Đặt biến môi trường rồi khởi động lại Summo."}
          {" "}
          Khoá không được lưu vào tệp cài đặt — nếu lưu, nó sẽ theo vào bản sao lưu và bản đồng bộ.
        </p>
      )}

      <div className="settings-actions">
        <button type="button" className="primary" onClick={() => void test()} disabled={testing}>
          {testing ? "Đang thử…" : "Thử kết nối"}
        </button>
        {status && <span className="field-hint">{status}</span>}
      </div>

      {result && (
        <p className={`banner ${result.ok ? "ok" : "error"}`} data-testid="test-result">
          {result.ok ? "Kết nối được" : "Không kết nối được"} — {result.base_url}
          <br />
          {result.local
            ? "Chạy trên máy bạn, không có gì rời khỏi máy."
            : "Văn bản bản ghi sẽ được gửi tới đây. Âm thanh thì không bao giờ."}
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
