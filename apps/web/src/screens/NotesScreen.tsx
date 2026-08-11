import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "../components/ui";
import { useErrorText } from "../lib/errors";
import { useI18n } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { NoteClient, SAVE_DEBOUNCE_MS, byDay, titleFrom, type NoteSummary } from "../lib/notes";

/**
 * Notes: a list on the left, the note on the right.
 *
 * Three decisions that shape how it feels.
 *
 * **There is no Save button.** A note is a file, and a file that only exists once you remember to
 * press something is a file people lose. It autosaves two seconds after you stop typing, and says
 * so quietly rather than flashing a toast at every pause.
 *
 * **The first line is the title.** Asking for a title in its own field before you may start typing
 * is the step that makes people close a note app and open a text editor. What you type becomes the
 * heading; the file is named after it.
 *
 * **The whole thing is one textarea, not a rich editor.** The vault is Markdown that a user opens in
 * Obsidian, greps and backs up; an editor that stores its own shape would break that promise the
 * first time it round-tripped something it did not understand. Tasks work here because
 * `- [ ] @ngoc …` is parsed from the file, not from a widget.
 */
export function NotesScreen() {
  const { handshake } = useEngine();
  const say = useErrorText();
  const { t } = useI18n();
  const client = useMemo(() => new NoteClient(handshake), [handshake]);

  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [saved, setSaved] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Held in a ref as well as in state: the debounce fires from a timer that closed over an older
  // render, and saving the text from two seconds ago would undo the last two seconds of typing.
  const latest = useRef(text);
  // In an effect rather than during render: mutating a ref while rendering is what React forbids,
  // and it buys nothing here — the timer that reads this fires later than any effect.
  useEffect(() => {
    latest.current = text;
  }, [text]);
  const timer = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      setNotes(await client.list());
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const open = useCallback(
    async (id: string) => {
      setOpenId(id);
      setSaved(true);
      try {
        const note = await client.read(id);
        // The title is the first line, so it is shown as the first line — the file stores them
        // apart, and stitching them back together is what makes the editor feel like one document.
        const body = note.body.trim();
        setText(body ? `${note.title}\n\n${body}` : note.title);
      } catch (e) {
        setError(say(e));
      }
    },
    [client, say],
  );

  const persist = useCallback(async () => {
    if (!openId) return;
    const { title, rest } = titleFrom(latest.current);
    try {
      await client.save(openId, rest, title);
      setSaved(true);
      // The list shows titles, and the one being edited has just changed.
      void refresh();
    } catch (e) {
      // Left as unsaved on purpose: telling the user it saved when it did not is how a note is
      // lost quietly rather than loudly.
      setError(say(e));
    }
  }, [client, openId, refresh, say]);

  const edit = (value: string) => {
    setText(value);
    setSaved(false);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => void persist(), SAVE_DEBOUNCE_MS);
  };

  // A note half-typed when the screen closes must not be lost to a timer that never fired.
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    [],
  );

  const create = async () => {
    setError(null);
    try {
      const { id } = await client.create(t("notes.untitled"));
      await refresh();
      await open(id);
    } catch (e) {
      setError(say(e));
    }
  };

  const remove = async (id: string) => {
    try {
      await client.remove(id);
      if (openId === id) {
        setOpenId(null);
        setText("");
      }
      await refresh();
    } catch (e) {
      setError(say(e));
    }
  };

  const grouped = byDay(notes);

  return (
    <div className="flex h-full min-h-0">
      <aside className="border-line flex w-72 shrink-0 flex-col border-r">
        <div className="border-line flex items-center justify-between gap-2 border-b px-3 py-2">
          <h1 className="text-sm font-semibold">{t("notes.title")}</h1>
          <Button size="sm" onClick={() => void create()}>
            {t("notes.new")}
          </Button>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          {notes.length === 0 && (
            <p className="text-fg-faint px-2 py-6 text-[13px]">{t("notes.empty")}</p>
          )}
          {grouped.map(([day, entries]) => (
            <section key={day} className="mb-3">
              <p className="text-fg-faint px-2 pb-1 text-[11px] font-semibold tracking-wider uppercase">
                {day}
              </p>
              <ul className="space-y-0.5">
                {entries.map((note) => (
                  <li key={note.id}>
                    <button
                      type="button"
                      onClick={() => void open(note.id)}
                      aria-current={note.id === openId}
                      className={`w-full truncate rounded-lg px-2 py-1.5 text-left text-sm ${
                        note.id === openId
                          ? "bg-accent-soft text-accent"
                          : "text-fg-dim hover:bg-bg-soft hover:text-fg"
                      }`}
                    >
                      {note.title}
                    </button>
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        {error && (
          <p
            role="alert"
            className="border-danger/30 bg-danger-soft text-danger border-b px-4 py-2 text-[13px]"
          >
            {error}
          </p>
        )}

        {openId === null ? (
          <p className="text-fg-faint mt-24 text-center">{t("notes.pick")}</p>
        ) : (
          <>
            <div className="border-line flex items-center gap-3 border-b px-4 py-2">
              <AnimatePresence mode="wait">
                <motion.span
                  key={saved ? "saved" : "editing"}
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  exit={{ opacity: 0 }}
                  className="text-fg-faint text-[12px]"
                >
                  {saved ? t("notes.saved") : t("notes.saving")}
                </motion.span>
              </AnimatePresence>
              <span className="flex-1" />
              <Button size="sm" variant="ghost" onClick={() => void remove(openId)}>
                {t("common.delete")}
              </Button>
            </div>

            <textarea
              value={text}
              onChange={(e) => edit(e.target.value)}
              onBlur={() => void persist()}
              aria-label={t("notes.body")}
              spellCheck={false}
              placeholder={t("notes.placeholder")}
              // `font-reading` and a measure: a note is prose, and prose at full window width is
              // prose nobody re-reads.
              className="font-reading min-h-0 flex-1 resize-none bg-transparent px-6 py-5 text-[15px] leading-relaxed outline-none"
            />
          </>
        )}
      </section>
    </div>
  );
}
