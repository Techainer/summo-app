import { useNavigate } from "@tanstack/react-router";
import { m } from "motion/react";
import { Check, CornerDownLeft, Loader, Sparkles, X } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { Markdown } from "../page/Markdown";

import { Button } from "../ui";
import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { url } from "../../lib/library";
import { GENTLE, listItem, stagger } from "../../lib/motion";

/** One thing said, and what came back. */
interface Turn {
  question: string;
  /** An answer read out of the vault, with the documents it came from. */
  answer?: { text: string; sources: { meeting: string; title: string; kind?: string }[] };
  /** An errand the agent carried out, with the steps it took. */
  errand?: { outcome: string; steps: { text: string; done: boolean }[] };
  error?: string;
}

/**
 * The assistant, beside whatever you are looking at.
 *
 * It was a screen you navigated *away* to, which is the wrong shape for the two things people
 * actually do with it: asking about the meeting currently on screen, and telling an agent to do
 * something while continuing to read. A panel keeps both halves visible.
 *
 * **Asking and telling are one box.** "What did Ngọc say about the budget" and "put the launch date
 * in the calendar" arrive the same way, and the difference — whether it reads or acts — is the
 * user's to make with a toggle rather than something to be guessed from grammar. Guessing wrong in
 * the acting direction writes to somebody's vault.
 *
 * Both halves cite. An answer names the documents it was drawn from; an errand lists the steps it
 * took and leaves them in Markdown in the day's scratch note, so the trace outlives the panel.
 */
export function AssistantPanel({ onClose }: { onClose: () => void }) {
  const t = useT();
  const navigate = useNavigate();
  const { handshake } = useEngine();

  const [turns, setTurns] = useState<Turn[]>([]);
  const [text, setText] = useState("");
  const [acting, setActing] = useState(false);
  const [busy, setBusy] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

  const send = useCallback(async () => {
    const said = text.trim();
    if (!said || busy) return;

    setText("");
    setBusy(true);
    setTurns((previous) => [...previous, { question: said }]);

    const settle = (patch: Partial<Turn>) =>
      setTurns((previous) =>
        previous.map((turn, index) =>
          index === previous.length - 1 ? { ...turn, ...patch } : turn,
        ),
      );

    try {
      const response = await fetch(url(handshake, acting ? "/agent/run" : "/ask"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(acting ? { instruction: said } : { question: said }),
      });
      const body = (await response.json()) as Record<string, unknown>;
      if (!response.ok) {
        throw new Error(typeof body.error === "string" ? body.error : response.statusText);
      }
      settle(acting ? { errand: body as never } : { answer: body as never });
    } catch (e) {
      settle({ error: e instanceof Error ? e.message : String(e) });
    } finally {
      setBusy(false);
      bottom.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [acting, busy, handshake, text]);

  return (
    <m.aside
      initial={{ x: 24, opacity: 0 }}
      animate={{ x: 0, opacity: 1 }}
      transition={GENTLE}
      aria-label={t("assistant.title")}
      data-testid="assistant"
      className="border-line bg-bg-soft flex h-full w-full flex-col border-s"
    >
      <header className="border-line flex items-center gap-2 border-b px-4 py-3">
        <Sparkles aria-hidden="true" className="text-ai size-4 shrink-0" />
        <h2 className="text-title font-semibold">{t("assistant.title")}</h2>
        <button
          type="button"
          onClick={onClose}
          aria-label={t("common.close")}
          className="text-fg-faint hover:bg-bg-raised hover:text-fg ms-auto rounded-lg p-1.5"
        >
          <X aria-hidden="true" className="size-4" />
        </button>
      </header>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto p-4">
        {turns.length === 0 && (
          <p className="text-fg-faint text-meta py-8 text-center">{t("assistant.intro")}</p>
        )}

        {turns.map((turn, index) => (
          <div key={`${turn.question}-${index}`} className="space-y-2">
            <p className="text-end">
              <span className="bg-accent-soft text-accent text-meta inline-block rounded-[var(--radius-panel)] px-3 py-1.5">
                {turn.question}
              </span>
            </p>

            {turn.error && (
              <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-[var(--radius-card)] border px-3 py-2">
                {turn.error}
              </p>
            )}

            {turn.answer && (
              <div className="border-line bg-bg-raised rounded-[var(--radius-card)] border p-3">
                {/* A model answers in Markdown — it writes lists and emphasis whether or not anyone
                    asked — so the answer is rendered rather than printed with its markers on. */}
                <Markdown markdown={turn.answer.text} className="text-body" />
                {turn.answer.sources.length > 0 && (
                  <div className="border-line mt-2.5 flex flex-wrap gap-1.5 border-t pt-2.5">
                    {turn.answer.sources.map((source) => (
                      <button
                        key={`${source.kind}-${source.meeting}`}
                        type="button"
                        onClick={() => {
                          onClose();
                          // Whichever kind it is. A cited note used to open `/notes` with nothing
                          // on it, so the control whose job is checking a claim was the one that
                          // dropped the evidence.
                          void navigate({
                            to: "/pages/$pageId",
                            params: { pageId: source.meeting },
                          });
                        }}
                        className="border-line text-fg-dim hover:border-accent/40 hover:text-accent text-micro rounded-[var(--radius-pill)] border px-2.5 py-1"
                      >
                        {source.title}
                      </button>
                    ))}
                  </div>
                )}
              </div>
            )}

            {/* The step log. This is the reference designs' execution list, and it is what makes an
                autonomous task legible instead of a spinner: a user who can see which step went
                wrong knows what to blame. */}
            {turn.errand && (
              <div className="border-ai/25 bg-ai-soft rounded-[var(--radius-card)] border p-3">
                <m.ul
                  initial="hidden"
                  animate="shown"
                  transition={stagger(turn.errand.steps.length)}
                  className="space-y-1"
                >
                  {turn.errand.steps.map((step, at) => (
                    <m.li
                      key={`${step.text}-${at}`}
                      variants={listItem}
                      className="text-meta flex items-baseline gap-2"
                    >
                      {step.done ? (
                        <Check aria-hidden="true" className="text-done size-3.5 shrink-0" />
                      ) : (
                        <Loader aria-hidden="true" className="text-ai size-3.5 shrink-0" />
                      )}
                      <span className={step.done ? "text-fg-dim" : "text-fg"}>{step.text}</span>
                    </m.li>
                  ))}
                </m.ul>
                {turn.errand.outcome && (
                  <p className="text-body border-ai/20 mt-2.5 border-t pt-2.5">
                    {turn.errand.outcome}
                  </p>
                )}
              </div>
            )}
          </div>
        ))}
        <div ref={bottom} />
      </div>

      <div className="border-line border-t p-3">
        {/* Read or act, chosen rather than guessed. Inferring "do it" from a sentence is inferring
            permission to write to somebody's vault. */}
        <div className="bg-bg-raised mb-2 flex gap-0.5 rounded-[var(--radius-pill)] p-0.5">
          {[
            { on: false, labelKey: "assistant.mode_ask" },
            { on: true, labelKey: "assistant.mode_do" },
          ].map((mode) => (
            <button
              key={mode.labelKey}
              type="button"
              aria-pressed={acting === mode.on}
              onClick={() => setActing(mode.on)}
              className={cn(
                "text-meta flex-1 rounded-[var(--radius-pill)] px-3 py-1 transition-colors",
                acting === mode.on
                  ? "bg-bg-elevated text-fg font-medium shadow-[var(--shadow-sm)]"
                  : "text-fg-dim hover:text-fg",
              )}
            >
              {t(mode.labelKey)}
            </button>
          ))}
        </div>

        <div className="flex items-end gap-2">
          <textarea
            value={text}
            rows={2}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey) {
                event.preventDefault();
                void send();
              }
            }}
            placeholder={t(acting ? "assistant.do_placeholder" : "assistant.ask_placeholder")}
            aria-label={t("assistant.title")}
            className="border-line bg-bg-raised text-body focus-visible:border-accent w-full resize-none rounded-[var(--radius-card)] border px-3 py-2 focus:outline-none"
          />
          <Button variant="primary" busy={busy} onClick={() => void send()}>
            <CornerDownLeft aria-hidden="true" className="size-3.5" />
          </Button>
        </div>
      </div>
    </m.aside>
  );
}
