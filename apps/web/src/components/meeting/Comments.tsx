import { AnimatePresence, m } from "motion/react";
import { useCallback, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useErrorText } from "../../lib/errors";
import { Markdown } from "../page/Markdown";
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
import { GENTLE, listItem } from "../../lib/motion";
import { useEngine } from "../../lib/engine-context";
import { Button } from "../ui";
import { useRefresh } from "../../lib/use-load";
import { X } from "lucide-react";

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
  const say = useErrorText();
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
      setError(say(e));
    }
  }, [client, say]);

  useRefresh(refresh);

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
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const react = async (id: string, emoji: string) => {
    try {
      await client.react(id, emoji, me);
      await refresh();
    } catch (e) {
      setError(say(e));
    }
  };

  const remove = async (id: string) => {
    try {
      await client.remove(id);
      await refresh();
    } catch (e) {
      setError(say(e));
    }
  };

  const ordered = inOrder(annotations);

  return (
    <section className="flex min-h-0 flex-col">
      <h2 className="text-fg-faint text-micro px-1 pb-2 font-semibold tracking-wider uppercase">
        {t("comments.title")}
      </h2>

      {error && (
        <p role="alert" className="text-danger text-meta mb-2">
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
              <m.li
                key={annotation.id}
                variants={listItem}
                initial="hidden"
                animate="shown"
                exit="gone"
                transition={GENTLE}
                className={`group rounded-xl border p-2.5 ${
                  fromAgent ? "border-accent/30 bg-accent-soft" : "border-line bg-bg-soft"
                }`}
              >
                <div className="text-micro flex items-baseline gap-2">
                  <b className="font-medium">{annotation.author}</b>
                  <span className="text-fg-faint">{writtenAt(annotation.at)}</span>
                  {label !== null && (
                    <button
                      type="button"
                      disabled={seq === null || !onSeek}
                      onClick={() => seq !== null && onSeek?.(seq)}
                      className="tabular bg-bg text-fg-dim enabled:hover:text-accent text-micro rounded-full px-1.5 disabled:cursor-default"
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
                    className="text-fg-faint hover:text-danger opacity-0 transition-opacity group-hover:opacity-100 focus:opacity-100"
                  >
                    <X aria-hidden="true" className="size-3.5" />
                  </button>
                </div>

                {/* An agent writes Markdown — lists, bold, links — and a person types text. So
                    only the agent's is rendered: silently reinterpreting what somebody typed into a
                    box with no formatting controls would be the app editing their words. */}
                {fromAgent ? (
                  <Markdown markdown={annotation.body} className="text-meta mt-1 space-y-2" />
                ) : (
                  <p className="text-meta mt-1 leading-relaxed whitespace-pre-wrap">
                    {annotation.body}
                  </p>
                )}

                <div className="mt-1.5 flex flex-wrap items-center gap-1">
                  {annotation.reactions?.map((reaction) => (
                    <button
                      key={reaction.emoji}
                      type="button"
                      onClick={() => void react(annotation.id, reaction.emoji)}
                      className={`text-micro rounded-full border px-1.5 py-0.5 ${
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
                      className="text-micro rounded-full px-1 opacity-0 transition-opacity group-hover:opacity-60 hover:!opacity-100 focus:opacity-100"
                    >
                      {emoji}
                    </button>
                  ))}
                </div>
              </m.li>
            );
          })}
        </AnimatePresence>

        {ordered.length === 0 && (
          <li className="text-fg-faint text-meta px-1 py-4">{t("comments.empty")}</li>
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
          className="border-line bg-bg-soft focus:border-accent text-meta min-w-0 flex-1 rounded-xl border px-3 py-1.5 outline-none"
        />
        <Button size="sm" onClick={() => void send()} disabled={!draft.trim() || busy}>
          {t("comments.send")}
        </Button>
      </div>
    </section>
  );
}
