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
        const body = (await response.json().catch(() => null)) as { error?: string } | null;
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
          <p className="mt-16 text-center text-fg-faint">
            Hỏi bất cứ điều gì đã được nói trong các buổi họp.
            <br />
            Câu trả lời luôn kèm buổi họp mà nó dựa vào.
          </p>
        )}

        {history.map((exchange, i) => (
          <div key={`${exchange.question}-${i}`} className="space-y-2">
            <p className="text-right">
              <span className="inline-block rounded-2xl bg-accent-soft px-3 py-1.5 text-sm text-accent">
                {exchange.question}
              </span>
            </p>

            {exchange.error && (
              <p className="rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
                {exchange.error}
              </p>
            )}

            {exchange.answer && (
              <Card>
                <CardBody className="pt-4">
                  <p className="whitespace-pre-wrap leading-relaxed">{exchange.answer.text}</p>
                  {exchange.answer.sources.length > 0 && (
                    <div className="mt-3 flex flex-wrap gap-1.5 border-t border-line pt-2.5">
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
                            "rounded-full border border-line px-2.5 py-1 text-[12px]",
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
              <p className="text-[13px] text-fg-faint">{t("chat.searching")}</p>
            )}
          </div>
        ))}
        <div ref={bottom} />
      </div>

      <form
        className="mt-3 flex gap-2 border-t border-line pt-3"
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
          className="flex-1 rounded-lg border border-line bg-bg-soft px-3 py-2 text-sm"
        />
        <Button variant="primary" type="submit" busy={busy}>
          Hỏi
        </Button>
      </form>
    </div>
  );
}
