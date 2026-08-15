import { useCallback, useState } from "react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { isRefinable, readable, selectionWithin, type Draft } from "../../lib/draft";
import { Button, Card, CardBody, CardHeader } from "../ui";

interface Props {
  draft: Draft;
  busy: boolean;
  onRefine: (heading: string, selection: string, instruction: string) => void;
  onChat: (message: string) => void;
  onConfirm: () => void;
  onDiscard: () => void;
}

/**
 * The agent's summary, before anyone has agreed to it.
 *
 * The sections are already in the note — this draws the same text tinted, so it is obvious at a
 * glance which paragraphs a model wrote. Confirming takes the tint off. Nothing moves.
 *
 * Two ways to change it, and they are offered differently on purpose. Selecting a passage brings up
 * a prompt box *at the selection*, because the user has already said where; that request rewrites
 * only that passage. The chat box at the bottom has no "where", so it revises the whole draft and
 * the result has to be re-read. Putting them in different places is what stops the cheap one being
 * used for the expensive job.
 */
export function DraftPanel({ draft, busy, onRefine, onChat, onConfirm, onDiscard }: Props) {
  const { t, n } = useI18n();
  const [picked, setPicked] = useState<{
    heading: string;
    text: string;
  } | null>(null);
  const [instruction, setInstruction] = useState("");
  const [message, setMessage] = useState("");

  const onSelect = useCallback((heading: string, element: HTMLElement | null) => {
    const text = selectionWithin(element);
    // A one-word selection is almost always a stray double-click.
    if (!text || !isRefinable(text)) {
      setPicked(null);
      return;
    }
    setPicked({ heading, text });
  }, []);

  const submitRefine = () => {
    if (!picked || !instruction.trim()) return;
    onRefine(picked.heading, picked.text, instruction.trim());
    setPicked(null);
    setInstruction("");
  };

  return (
    <Card className="border-accent/40">
      <CardHeader
        title={t("draft.heading")}
        count={draft.revisions > 0 ? n("draft.revised", draft.revisions) : t("draft.pending")}
        actions={
          <>
            <Button size="sm" variant="ghost" onClick={onDiscard} disabled={busy}>
              {t("draft.discard")}
            </Button>
            <Button size="sm" variant="primary" onClick={onConfirm} busy={busy}>
              {t("draft.confirm")}
            </Button>
          </>
        }
      />

      <CardBody className="space-y-4">
        <p className="text-fg-faint text-micro">{t("draft.select_hint")}</p>

        {draft.sections.map((section) => (
          <section key={section.heading}>
            <h3 className="text-fg-dim text-meta font-semibold">{section.heading}</h3>
            {/* Not a control, despite the handlers. What is being listened for is a *selection*
                — the user dragging or shift-arrowing across a phrase to comment on it — and both
                pointers and keyboards are covered, which is what the rule protects. Making this a
                button would be a lie about what it does, and would take the text out of the
                reading order it belongs in. */}
            {/* eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions */}
            <p
              // The tint is the whole signal: this text is in the note but nobody has agreed to it.
              className="bg-accent-soft selection:bg-accent selection:text-accent-fg mt-1 rounded-md px-2 py-1.5 leading-relaxed whitespace-pre-wrap"
              onMouseUp={(e) => onSelect(section.heading, e.currentTarget)}
              onKeyUp={(e) => onSelect(section.heading, e.currentTarget)}
            >
              {readable(section.body)}
            </p>
          </section>
        ))}

        {picked && (
          <div className="border-accent/40 bg-bg-soft rounded-[var(--radius-card)] border p-2.5">
            <p className="text-fg-dim text-micro">
              {t("draft.revising", { heading: picked.heading })}{" "}
              <span className="italic">“{shorten(picked.text)}”</span>
            </p>
            <form
              className="mt-2 flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                submitRefine();
              }}
            >
              <input
                autoFocus
                value={instruction}
                onChange={(e) => setInstruction(e.target.value)}
                placeholder={t("draft.revise_placeholder")}
                aria-label={t("draft.revise_label")}
                disabled={busy}
                className="border-line bg-bg flex-1 rounded-lg border px-2.5 py-1.5 text-sm"
              />
              <Button size="sm" variant="primary" type="submit" busy={busy}>
                {t("draft.apply")}
              </Button>
              <Button size="sm" variant="ghost" type="button" onClick={() => setPicked(null)}>
                {t("draft.cancel")}
              </Button>
            </form>
          </div>
        )}

        {draft.turns.length > 0 && (
          <ol className="border-line space-y-1.5 border-t pt-3">
            {draft.turns.map((turn, i) => (
              <li
                key={`${turn.role}-${i}`}
                className={cn("text-meta", turn.role === "you" ? "text-fg" : "text-fg-faint")}
              >
                <span className="font-medium">
                  {turn.role === "you" ? t("draft.you") : t("draft.agent")}:{" "}
                </span>
                {turn.text}
              </li>
            ))}
          </ol>
        )}

        <form
          className="border-line flex gap-2 border-t pt-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (!message.trim()) return;
            onChat(message.trim());
            setMessage("");
          }}
        >
          <input
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder={t("draft.chat_placeholder")}
            aria-label={t("draft.chat_send")}
            disabled={busy}
            className="border-line bg-bg flex-1 rounded-lg border px-2.5 py-1.5 text-sm"
          />
          <Button size="sm" type="submit" busy={busy}>
            {t("draft.send")}
          </Button>
        </form>
      </CardBody>
    </Card>
  );
}

function shorten(text: string): string {
  const trimmed = text.trim();
  return trimmed.length <= 60 ? trimmed : `${trimmed.slice(0, 60)}…`;
}
