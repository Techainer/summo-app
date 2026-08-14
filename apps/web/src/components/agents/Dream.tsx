import { Moon } from "lucide-react";
import { useCallback, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { readJson } from "../../lib/errors";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { Button } from "../ui";

interface State {
  dream: boolean;
  hour: number;
  last: {
    day: string;
    agents: { agent: string; before: number; after: number; refused?: string }[];
  } | null;
}

/**
 * Let the agents sleep on it.
 *
 * Memory only ever grew: the same fact written three ways, a correction sitting above the thing it
 * corrected, a note about a project that finished in March — all of it in every prompt, forever.
 * Once a night the agent re-reads what it knows and writes back a shorter version that says the
 * same things.
 *
 * Off unless asked, because it is a language-model call per agent per day, and it says what it did:
 * how many lines went in, how many came out, or why the night was thrown away. The previous memory
 * is kept in `DREAMS.md` beside the agent, so a bad night is one file to open and one block to
 * paste back.
 */
export function DreamPanel() {
  const { handshake } = useEngine();
  const { t } = useI18n();
  const say = useErrorText();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const state = useLoad(
    useCallback(
      async () => readJson<State>(await fetch(url(handshake, "/agent/dream"))),
      [handshake],
    ),
    [handshake],
  );

  const send = async (body: Record<string, unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await readJson(
        await fetch(url(handshake, "/agent/dream"), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body),
        }),
      );
      state.reload();
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const on = state.data?.dream ?? false;
  const last = state.data?.last ?? null;

  return (
    <section
      className="border-line bg-bg-soft mt-6 rounded-[var(--radius-panel)] border p-4"
      data-testid="dream"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Moon className="text-ai size-4 shrink-0" aria-hidden="true" />
        <h2 className="flex-1 text-sm font-semibold">{t("agents.dream")}</h2>
        <label className="text-meta flex items-center gap-1.5">
          <input
            type="checkbox"
            checked={on}
            disabled={busy}
            onChange={(e) => void send({ dream: e.target.checked })}
            className="accent-[var(--color-accent)]"
          />
          {t("agents.dream_nightly", { hour: String(state.data?.hour ?? 3) })}
        </label>
        <Button variant="ghost" size="sm" disabled={busy} onClick={() => void send({ now: true })}>
          {busy ? t("agents.dream_working") : t("agents.dream_now")}
        </Button>
      </div>
      <p className="text-fg-faint text-micro mt-1">{t("agents.dream_hint")}</p>

      {error && (
        <p role="alert" className="text-danger mt-2 text-sm">
          {error}
        </p>
      )}

      {last && (
        <ul className="text-meta mt-3 space-y-1" data-testid="dream-last">
          {last.agents.map((one) => (
            <li key={one.agent} className="text-fg-dim">
              <span className="font-medium">{one.agent}</span>
              {" — "}
              {one.refused
                ? t("agents.dream_refused", { why: one.refused })
                : t("agents.dream_shrank", {
                    before: String(one.before),
                    after: String(one.after),
                  })}
            </li>
          ))}
          {last.agents.length === 0 && <li className="text-fg-faint">{t("agents.dream_none")}</li>}
        </ul>
      )}
    </section>
  );
}
