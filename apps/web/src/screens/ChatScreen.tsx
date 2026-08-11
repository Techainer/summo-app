import { useNavigate } from "@tanstack/react-router";
import { useCallback, useRef, useState } from "react";

import { Button, Card, CardBody } from "../components/ui";
import { cn } from "../lib/cn";
import { useT } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { url } from "../lib/library";

interface Source {
  meeting: string;
  title: string;
  day: string;
}

interface Answer {
  question: string;
  text: string;
  sources: Source[];
}

interface Exchange {
  question: string;
  answer?: Answer;
  error?: string;
}

/**
 * Asking the vault a question.
 *
 * Every answer carries the meetings it came from, and they are links. That is not decoration: the
 * model is instructed to answer only from the excerpts it was shown, and the way a user finds out
 * whether it obeyed is by opening the meeting and reading the line.
 */
export function ChatScreen() {
  const t = useT();
  const { handshake } = useEngine();
  const navigate = useNavigate();
  const [history, setHistory] = useState<Exchange[]>([]);
  const [question, setQuestion] = useState("");
  const [busy, setBusy] = useState(false);
  const bottom = useRef<HTMLDivElement>(null);

  const send = useCallback(async () => {
    const asked = question.trim();
    if (!asked || busy) return;

    setQuestion("");
    setBusy(true);
    setHistory((h) => [...h, { question: asked }]);

    try {
      const response = await fetch(url(handshake, "/ask"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ question: asked }),
      });
      if (!response.ok) {
        const body = (await response.json().catch(() => null)) as {
          error?: string;
        } | null;
        throw new Error(body?.error ?? `${response.status}`);
      }
      const answer = (await response.json()) as Answer;
      setHistory((h) => h.map((e, i) => (i === h.length - 1 ? { ...e, answer } : e)));
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      setHistory((h) => h.map((x, i) => (i === h.length - 1 ? { ...x, error: message } : x)));
    } finally {
      setBusy(false);
      bottom.current?.scrollIntoView({ behavior: "smooth" });
    }
  }, [question, busy, handshake]);

  return (
    <div className="mx-auto flex h-full max-w-3xl flex-col p-5">
      <h1 className="text-xl font-semibold tracking-tight">{t("chat.heading")}</h1>

      <div className="mt-4 min-h-0 flex-1 space-y-4 overflow-y-auto">
        {history.length === 0 && (
          <p className="text-fg-faint mt-16 text-center">
            {t("chat.intro")}
            <br />
            {t("chat.intro_2")}
          </p>
        )}

        {history.map((exchange, i) => (
          <div key={`${exchange.question}-${i}`} className="space-y-2">
            <p className="text-right">
              <span className="bg-accent-soft text-accent inline-block rounded-2xl px-3 py-1.5 text-sm">
                {exchange.question}
              </span>
            </p>

            {exchange.error && (
              <p className="border-rec/30 bg-rec-soft text-rec rounded-lg border px-3 py-2 text-[13px]">
                {exchange.error}
              </p>
            )}

            {exchange.answer && (
              <Card>
                <CardBody className="pt-4">
                  <p className="leading-relaxed whitespace-pre-wrap">{exchange.answer.text}</p>
                  {exchange.answer.sources.length > 0 && (
                    <div className="border-line mt-3 flex flex-wrap gap-1.5 border-t pt-2.5">
                      {exchange.answer.sources.map((source) => (
                        <button
                          key={source.meeting}
                          type="button"
                          onClick={() =>
                            void navigate({
                              to: "/meetings/$meetingId",
                              params: { meetingId: source.meeting },
                            })
                          }
                          className={cn(
                            "border-line rounded-full border px-2.5 py-1 text-[12px]",
                            "text-fg-dim hover:border-accent/40 hover:text-accent",
                          )}
                        >
                          {source.title} · {source.day}
                        </button>
                      ))}
                    </div>
                  )}
                </CardBody>
              </Card>
            )}

            {!exchange.answer && !exchange.error && (
              <p className="text-fg-faint text-[13px]">{t("chat.searching")}</p>
            )}
          </div>
        ))}
        <div ref={bottom} />
      </div>

      <form
        className="border-line mt-3 flex gap-2 border-t pt-3"
        onSubmit={(e) => {
          e.preventDefault();
          void send();
        }}
      >
        <input
          value={question}
          onChange={(e) => setQuestion(e.target.value)}
          placeholder={t("chat.placeholder")}
          aria-label={t("chat.question")}
          disabled={busy}
          className="border-line bg-bg-soft flex-1 rounded-lg border px-3 py-2 text-sm"
        />
        <Button variant="primary" type="submit" busy={busy}>
          {t("chat.ask")}
        </Button>
      </form>
    </div>
  );
}
