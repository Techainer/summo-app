import { useCallback, useState } from "react";

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
  const [picked, setPicked] = useState<{ heading: string; text: string } | null>(null);
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
        title="Bản tóm tắt agent viết"
        count={draft.revisions > 0 ? `đã sửa ${draft.revisions} lần` : "chưa duyệt"}
        actions={
          <>
            <Button size="sm" variant="ghost" onClick={onDiscard} disabled={busy}>
              Bỏ
            </Button>
            <Button size="sm" variant="primary" onClick={onConfirm} busy={busy}>
              Xác nhận
            </Button>
          </>
        }
      />

      <CardBody className="space-y-4">
        <p className="text-[12px] text-fg-faint">
          Bôi đen một đoạn để sửa riêng đoạn đó, hoặc nhắn ở dưới để sửa cả bài.
        </p>

        {draft.sections.map((section) => (
          <section key={section.heading}>
            <h3 className="text-[13px] font-semibold text-fg-dim">{section.heading}</h3>
            <p
              // The tint is the whole signal: this text is in the note but nobody has agreed to it.
              className="mt-1 whitespace-pre-wrap rounded-md bg-accent-soft px-2 py-1.5 leading-relaxed
                selection:bg-accent selection:text-accent-fg"
              onMouseUp={(e) => onSelect(section.heading, e.currentTarget)}
              onKeyUp={(e) => onSelect(section.heading, e.currentTarget)}
            >
              {readable(section.body)}
            </p>
          </section>
        ))}

        {picked && (
          <div className="rounded-[var(--radius-card)] border border-accent/40 bg-bg-soft p-2.5">
            <p className="text-[12px] text-fg-dim">
              Sửa trong <strong>{picked.heading}</strong>:{" "}
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
                placeholder="ngắn hơn, bỏ tên khách, thêm mốc thời gian…"
                aria-label="Muốn sửa thế nào"
                disabled={busy}
                className="flex-1 rounded-lg border border-line bg-bg px-2.5 py-1.5 text-sm"
              />
              <Button size="sm" variant="primary" type="submit" busy={busy}>
                Sửa
              </Button>
              <Button size="sm" variant="ghost" type="button" onClick={() => setPicked(null)}>
                Huỷ
              </Button>
            </form>
          </div>
        )}

        {draft.turns.length > 0 && (
          <ol className="space-y-1.5 border-t border-line pt-3">
            {draft.turns.map((turn, i) => (
              <li
                key={`${turn.role}-${i}`}
                className={cn(
                  "text-[13px]",
                  turn.role === "you" ? "text-fg" : "text-fg-faint",
                )}
              >
                <span className="font-medium">{turn.role === "you" ? "Bạn" : "Agent"}: </span>
                {turn.text}
              </li>
            ))}
          </ol>
        )}

        <form
          className="flex gap-2 border-t border-line pt-3"
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
            placeholder="Nhắn cho agent để sửa cả bản tóm tắt…"
            aria-label="Nhắn cho agent"
            disabled={busy}
            className="flex-1 rounded-lg border border-line bg-bg px-2.5 py-1.5 text-sm"
          />
          <Button size="sm" type="submit" busy={busy}>
            Gửi
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
