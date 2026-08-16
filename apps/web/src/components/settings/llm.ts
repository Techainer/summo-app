import { useCallback, useEffect, useState } from "react";

import { useT } from "../../i18n/context";
import { url } from "../../lib/library";
import type { Handshake } from "../../lib/engine";

/**
 * The language-model settings, held once for the two sections that show them.
 *
 * Summarising and translating are two different jobs with two different models — a 1B translation
 * model beats a general 8B one at translating, and the general one is the only thing that can write
 * a summary — so they are two sections on screen. They are one record on disk, and one save: state
 * split between two panels is how a screen ends up sending a stale translator alongside a fresh
 * provider and quietly undoing half of what the user just did.
 */

/**
 * One endpoint, as the daemon describes it.
 *
 * The list used to be a hardcoded array of four here, alongside the daemon's own list of four.
 * Adding a provider meant editing both, in two languages, and two facts could not be shown at all
 * because only the daemon had them: which environment variable holds the key, and whether it is
 * already set on this machine.
 */
export interface ProviderInfo {
  id: string;
  name: string;
  base_url: string;
  model: string;
  local: boolean;
  key_env: string | null;
  key_set: boolean;
}

export interface Translator {
  provider: string;
  model: string | null;
}

export interface Llm {
  provider: string;
  model: string | null;
  language: string;
  summarize_on_stop: boolean;
  /** A second, smaller model that does translation only. `null` sends translation to the one above. */
  translator?: Translator | null;
}

export interface TestResult {
  ok: boolean;
  base_url: string;
  local: boolean;
  detail: string;
}

/** The pseudo-entry for "some other OpenAI-compatible server". Not a preset; there is no list of them. */
export const CUSTOM = "custom";

/**
 * The translator provider that means "inside Summo".
 *
 * Matches `summo_core::settings::LOCAL`. Deliberately not a URL: it is the *absence* of an endpoint,
 * and the daemon must never try to resolve it as one.
 */
export const LOCAL = "local";

export interface LlmSettings {
  llm: Llm | null;
  providers: ProviderInfo[];
  keyPresent: boolean;
  custom: string;
  setCustom: (value: string) => void;
  /** Local edit without a round trip, for a field that saves on blur. */
  edit: (next: Llm) => void;
  save: (next: Llm) => Promise<void>;
  test: () => Promise<void>;
  testing: boolean;
  result: TestResult | null;
  status: string | null;
}

export function useLlm(handshake: Handshake): LlmSettings {
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

  return {
    llm,
    providers,
    keyPresent,
    custom,
    setCustom,
    edit: setLlm,
    save,
    test,
    testing,
    result,
    status,
  };
}
