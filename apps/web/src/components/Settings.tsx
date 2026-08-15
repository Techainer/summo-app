import { Checkbox, Input, Page, SegmentedControl } from "./ui";
import { useCallback, useEffect, useState } from "react";

import { useI18n, useT } from "../i18n/context";
import { About } from "./About";
import { Permissions } from "./onboarding/Permissions";
import { SpokenLanguage } from "./record/SpokenLanguage";
import { load as loadCapture, save as saveCapture } from "../lib/capture";
import { url } from "../lib/library";
import type { Handshake } from "../lib/engine";
import { SCHEMES, choose as chooseScheme, read as readScheme, type Scheme } from "../lib/theme";

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
const FIELD = "mt-3.5 flex items-center gap-3 text-meta text-fg-dim";
const LABEL = "w-[150px] shrink-0";
/** Settings controls are `Input`s that stretch; the shared field owns everything else about them. */
const CONTROL = "flex-1";
/** A native `<select>`, which cannot be an `Input` — same box, drawn by hand. */
const SELECT =
  "h-9 flex-1 rounded-[var(--radius-card)] border border-line bg-bg-soft px-3 text-sm text-fg" +
  " transition-colors hover:border-line-strong focus-visible:border-accent focus:outline-none";
/** The note under a field. Indented to line up with the control it explains. */
const HINT = "mt-1.5 ml-[162px] text-micro leading-normal text-fg-faint";

/** The pseudo-entry for "some other OpenAI-compatible server". Not a preset; there is no list of them. */
const CUSTOM = "custom";

/**
 * The translator provider that means "inside Summo".
 *
 * Matches `summo_core::settings::LOCAL`. Deliberately not a URL: it is the *absence* of an
 * endpoint, and the daemon must never try to resolve it as one.
 */
const LOCAL = "local";

interface Llm {
  provider: string;
  model: string | null;
  language: string;
  summarize_on_stop: boolean;
  /** A second, smaller model that does translation only. `null` sends translation to the one above. */
  translator?: Translator | null;
}

interface Translator {
  provider: string;
  model: string | null;
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
      if (!response.ok) {
        // `parsed.error` is `unknown`: a daemon that answered with an object would otherwise be
        // reported to the user as "[object Object]", which is worse than saying nothing.
        const reason = typeof parsed.error === "string" ? parsed.error : response.statusText;
        throw new Error(reason);
      }
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
    [post, t],
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

  if (!llm)
    return (
      <p className="text-fg-faint grid h-full place-items-center px-6 text-center">
        {status ?? t("settings.loading")}
      </p>
    );

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
    <Page width="narrow" data-testid="settings">
      <LanguagePicker />
      <AppearanceSetting />

      {/* Above the model and provider settings, because it is the one that stops a recording
          outright — and because a user who arrives here after a failed recording is looking for
          exactly this. */}
      <div className="my-6">
        <Permissions />
      </div>

      <SpokenLanguageSetting />

      <h2 className="text-xl font-semibold tracking-tight">{t("settings.llm_heading")}</h2>
      <p className="text-fg-faint text-meta my-4 leading-normal">
        {t("settings.llm_hint_head")}
        <b>{t("settings.always_local")}</b>
        {t("settings.llm_hint_tail")}
      </p>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.provider")}</span>
        <select
          className={SELECT}
          value={selected}
          aria-label={t("settings.provider")}
          onChange={(e) => {
            const value = e.target.value;
            void save({
              ...llm,
              provider: value === "custom" ? custom || "http://" : value,
            });
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
          onChange={(e) => setLlm({ ...llm, model: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <label className={FIELD}>
        <span className={LABEL}>{t("settings.summary_language")}</span>
        <Input
          className={CONTROL}
          value={llm.language}
          aria-label={t("settings.summary_language")}
          onChange={(e) => setLlm({ ...llm, language: e.target.value })}
          onBlur={() => void save(llm)}
        />
      </label>

      <Checkbox
        className="mt-3.5"
        checked={llm.summarize_on_stop}
        onChange={(summarize_on_stop) => void save({ ...llm, summarize_on_stop })}
      >
        {t("settings.summarise_on_stop")}
      </Checkbox>

      {/* Translation gets its own model because it is a different job. A 1B translation model
          beats a general 8B one at it, runs on the CPU in under a second a line, and costs
          nothing — which is what lets someone with no API key at all still translate every
          meeting they record. */}
      <h2 className="mt-8 text-xl font-semibold tracking-tight">{t("settings.mt_heading")}</h2>
      <p className="text-fg-faint text-meta my-4 leading-normal">{t("settings.mt_hint")}</p>

      <Checkbox
        className="mt-3.5"
        checked={llm.translator != null}
        onChange={(on) =>
          void save({
            ...llm,
            // Turning it on proposes the in-app model, not an endpoint. "Enable this, now go and
            // install a model server" is not a setting anybody finishes, and the whole claim of
            // this feature is that translation costs nothing — which stops being true the moment
            // it depends on a second program.
            translator: on ? { provider: LOCAL, model: "small100" } : null,
          })
        }
      >
        {t("settings.mt_enable")}
      </Checkbox>

      {llm.translator != null && (
        <>
          <label className={FIELD}>
            <span className={LABEL}>{t("settings.mt_where")}</span>
            <select
              className={SELECT}
              value={llm.translator.provider === LOCAL ? LOCAL : "endpoint"}
              aria-label={t("settings.mt_where")}
              onChange={(e) =>
                void save({
                  ...llm,
                  translator:
                    e.target.value === LOCAL
                      ? { provider: LOCAL, model: "small100" }
                      : { provider: "llama-cpp", model: "milmmt-46-1b" },
                })
              }
            >
              <option value={LOCAL}>{t("settings.mt_in_app")}</option>
              <option value="endpoint">{t("settings.mt_endpoint")}</option>
            </select>
          </label>

          {llm.translator.provider !== LOCAL && (
            <label className={FIELD}>
              <span className={LABEL}>{t("settings.endpoint")}</span>
              <Input
                className={CONTROL}
                value={llm.translator.provider}
                aria-label={t("settings.mt_endpoint")}
                placeholder="llama-cpp"
                onChange={(e) =>
                  setLlm({
                    ...llm,
                    translator: { model: llm.translator?.model ?? null, provider: e.target.value },
                  })
                }
                onBlur={() => void save(llm)}
              />
            </label>
          )}

          <label className={FIELD}>
            <span className={LABEL}>{t("settings.model")}</span>
            <Input
              className={CONTROL}
              value={llm.translator.model ?? ""}
              aria-label={t("settings.mt_model")}
              placeholder="small100"
              onChange={(e) =>
                setLlm({
                  ...llm,
                  translator: {
                    provider: llm.translator?.provider ?? LOCAL,
                    model: e.target.value,
                  },
                })
              }
              onBlur={() => void save(llm)}
            />
          </label>

          <p className={HINT}>
            {llm.translator.provider === LOCAL ? t("settings.mt_pull") : t("settings.mt_run")}
          </p>
        </>
      )}

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
      <About />
    </Page>
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
      <p className="text-fg-faint text-meta my-4 leading-normal">{t("settings.language_hint")}</p>
    </>
  );
}

/**
 * The language being spoken, as a standing preference.
 *
 * The same control the record bar carries, because it is the same decision — but it belongs here
 * too: the record bar is where you change it for *this* meeting, and this is where you change what
 * every meeting starts from. Somebody who records in one language and has to reselect it every
 * time is being asked a question that was already answered.
 *
 * `SpokenLanguage` writes through to the daemon itself; this only has to keep the browser's own
 * copy in step, so the record bar opens on the same answer.
 */
function SpokenLanguageSetting() {
  const t = useT();
  const [spoken, setSpoken] = useState(() => loadCapture().spoken);

  return (
    <section className="border-line bg-bg-raised my-6 rounded-2xl border p-5">
      <h2 className="font-medium">{t("settings.spoken_heading")}</h2>
      <p className="text-fg-dim text-meta mt-1 mb-3">{t("settings.spoken_hint")}</p>
      <SpokenLanguage
        value={spoken}
        onChange={(code) => {
          setSpoken(code);
          saveCapture({ ...loadCapture(), spoken: code });
        }}
      />
    </section>
  );
}

/**
 * Light, dark, or whatever the machine says.
 *
 * Here as well as in ⌘K, because a preference that only exists in a command palette is one most
 * people never find — and because this is the screen somebody opens when they are looking for a
 * setting rather than for a shortcut. `theme.css` has had both blocks written since the palette was
 * rebuilt; nothing set the attribute, so dark mode has only ever followed the operating system.
 */
function AppearanceSetting() {
  const t = useT();
  const [scheme, setScheme] = useState<Scheme>(() => readScheme());

  return (
    <div className={FIELD}>
      <span className={LABEL}>{t("theme.heading")}</span>
      <SegmentedControl
        label={t("theme.heading")}
        value={scheme}
        onChange={(next) => {
          chooseScheme(next);
          setScheme(next);
        }}
        // Short labels here, the sentence in the palette. Three phrases like "Giao diện theo hệ
        // thống" side by side in a 390px column is a control that wraps out of its own pill — the
        // screenshot audit caught it as white text on nothing.
        options={SCHEMES.map((one) => ({ value: one, label: t(`theme.short_${one}`) }))}
      />
    </div>
  );
}
