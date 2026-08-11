import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import {
  CommentClient,
  QUICK,
  anchorLabel,
  inOrder,
  reacted,
  segmentOf,
  writtenAt,
  type Annotation,
} from "../../lib/comments";
import { useEngine } from "../../lib/engine-context";
import { Button } from "../ui";

/**
 * The conversation about a meeting, beside the meeting.
 *
 * The agent's proposals live in the same thread as people's comments, deliberately. A comment and
 * "shall I add this as a task?" are the same conversation; splitting them into two panels makes the
 * agent something you check on rather than something you talk to.
 *
 * A comment pinned to an utterance seeks the player when clicked, which is the whole reason to pin
 * one — "Ngọc said something different at 12:04" is only useful if 12:04 is one click away.
 */
export function Comments({
  meeting,
  onSeek,
}: {
  meeting: string;
  /** Jump the player to an utterance. Absent when there is no audio to jump in. */
  onSeek?: (seq: number) => void;
}) {
  const { handshake } = useEngine();
  const { t } = useI18n();
  const client = useMemo(() => new CommentClient(handshake, meeting), [handshake, meeting]);

  const [annotations, setAnnotations] = useState<Annotation[]>([]);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Whoever is at this machine. The vault is one person's, so there is no account to read this
  // from — and asking for a name before somebody may leave a comment is a form nobody fills in.
  const me = t("comments.me");

  const refresh = useCallback(async () => {
    try {
      const thread = await client.list();
      setAnnotations(thread.annotations);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const send = async () => {
    const body = draft.trim();
    if (!body) return;
    setBusy(true);
    setError(null);
    try {
      await client.add(body, me);
      setDraft("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  const react = async (id: string, emoji: string) => {
    try {
      await client.react(id, emoji, me);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const remove = async (id: string) => {
    try {
      await client.remove(id);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const ordered = inOrder(annotations);

  return (
    <section className="flex min-h-0 flex-col">
      <h2 className="px-1 pb-2 text-[11px] font-semibold uppercase tracking-wider text-fg-faint">
        {t("comments.title")}
      </h2>

      {error && (
        <p role="alert" className="mb-2 text-[13px] text-danger">
          {error}
        </p>
      )}

      <ul className="min-h-0 flex-1 space-y-2 overflow-y-auto">
        <AnimatePresence initial={false}>
          {ordered.map((annotation) => {
            const seq = segmentOf(annotation.anchor);
            const label = anchorLabel(annotation.anchor);
            const fromAgent = annotation.author === "agent";

            return (
              <motion.li
                key={annotation.id}
                initial={{ opacity: 0, y: -3 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                className={`group rounded-xl border p-2.5 ${
                  fromAgent ? "border-accent/30 bg-accent-soft" : "border-line bg-bg-soft"
                }`}
              >
                <div className="flex items-baseline gap-2 text-[12px]">
                  <b className="font-medium">{annotation.author}</b>
                  <span className="text-fg-faint">{writtenAt(annotation.at)}</span>
                  {label !== null && (
                    <button
                      type="button"
                      disabled={seq === null || !onSeek}
                      onClick={() => seq !== null && onSeek?.(seq)}
                      className="tabular rounded-full bg-bg px-1.5 text-[11px] text-fg-dim enabled:hover:text-accent disabled:cursor-default"
                    >
                      {label}
                    </button>
                  )}
                  <span className="flex-1" />
                  <button
                    type="button"
                    aria-label={t("comments.remove")}
                    onClick={() => void remove(annotation.id)}
                    // Revealed on hover: a delete button on every comment, always visible, is a
                    // thread that looks like a list of things to get rid of.
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100 text-fg-faint hover:text-danger"
                  >
                    ✕
                  </button>
                </div>

                <p className="mt-1 whitespace-pre-wrap text-[13px] leading-relaxed">
                  {annotation.body}
                </p>

                <div className="mt-1.5 flex flex-wrap items-center gap-1">
                  {annotation.reactions?.map((reaction) => (
                    <button
                      key={reaction.emoji}
                      type="button"
                      onClick={() => void react(annotation.id, reaction.emoji)}
                      className={`rounded-full border px-1.5 py-0.5 text-[12px] ${
                        reaction.by.includes(me)
                          ? "border-accent bg-accent-soft text-accent"
                          : "border-line"
                      }`}
                    >
                      {reaction.emoji} {reaction.by.length}
                    </button>
                  ))}

                  {QUICK.filter((emoji) => !reacted(annotation, emoji, me)).map((emoji) => (
                    <button
                      key={emoji}
                      type="button"
                      aria-label={t("comments.react", { emoji })}
                      onClick={() => void react(annotation.id, emoji)}
                      className="rounded-full px-1 text-[12px] opacity-0 transition-opacity group-hover:opacity-60 hover:!opacity-100 focus:opacity-100"
                    >
                      {emoji}
                    </button>
                  ))}
                </div>
              </motion.li>
            );
          })}
        </AnimatePresence>

        {ordered.length === 0 && (
          <li className="px-1 py-4 text-[13px] text-fg-faint">{t("comments.empty")}</li>
        )}
      </ul>

      <div className="mt-2 flex gap-2">
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void send();
            }
          }}
          placeholder={t("comments.placeholder")}
          aria-label={t("comments.title")}
          className="min-w-0 flex-1 rounded-xl border border-line bg-bg-soft px-3 py-1.5 text-[13px] outline-none focus:border-accent"
        />
        <Button size="sm" onClick={() => void send()} disabled={!draft.trim() || busy}>
          {t("comments.send")}
        </Button>
      </div>
    </section>
  );
}
