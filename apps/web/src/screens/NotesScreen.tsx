import { NotebookPen } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { Button, Empty } from "../components/ui";
import { NoteEditor } from "../components/page/NoteEditor";
import { useErrorText } from "../lib/errors";
import { useSearch } from "@tanstack/react-router";

import { useI18n } from "../i18n/context";
import { Dot } from "../components/library/Finder";
import { useEngine } from "../lib/engine-context";
import { NoteClient, byDay, type NoteSummary } from "../lib/notes";
import { useRefresh } from "../lib/use-load";

/**
 * Notes: a list on the left, the note on the right.
 *
 * A workspace for the kind of writing that was never recorded — the notes only, by day, with their
 * colours. The sidebar tree lists the same documents by *folder*, alongside the recordings, which
 * is a different question about the same set and the reason both exist.
 *
 * The editor itself is [`NoteEditor`], shared with the page screen, so a note opened from here and
 * a note opened from the tree are the same editor rather than two that drift.
 */
/**
 * The shapes a note can start in. `blank` first, because most notes are.
 */
const KINDS = ["blank", "idea", "decision", "todo", "journal"] as const;
type NoteKind = (typeof KINDS)[number];

export function NotesScreen() {
  const { handshake } = useEngine();
  const say = useErrorText();
  const { t } = useI18n();
  const client = useMemo(() => new NoteClient(handshake), [handshake]);

  const [notes, setNotes] = useState<NoteSummary[]>([]);
  const [openId, setOpenId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setNotes(await client.list());
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useRefresh(refresh);

  // Opened from a link somebody kept. `?open=` predates the page screen, which is where the tree
  // and every citation now go; it stays because a URL that used to work should go on working.
  //
  // Adjusted during render rather than in an effect. An effect that sets state from a prop renders
  // the screen once with the old note and once with the new one, and the first of those two frames
  // shows the wrong document — which is precisely the frame somebody following a link is looking
  // at. React sanctions this shape for exactly this case.
  const wanted = useSearch({ from: "/notes" }).open;
  const [linked, setLinked] = useState(wanted);
  if (wanted !== linked) {
    setLinked(wanted);
    if (wanted) setOpenId(wanted);
  }

  /**
   * A new note, optionally with a shape already in it.
   *
   * A blank page is the right default and a poor only option: "ý tưởng", "quyết định", "việc cần
   * làm" and a day's journal are what people were typing headings for by hand, and each one has a
   * different set of headings. Seeded rather than templated in the daemon — the seed is ordinary
   * Markdown in the note from the first keystroke, so nothing about it can go stale or need
   * migrating, and deleting the headings is how you opt out.
   */
  const create = async (kind: NoteKind = "blank") => {
    setError(null);
    setPicking(false);
    try {
      const body = kind === "blank" ? "" : t(`notes.seed_${kind}`);
      const { id } = await client.create(
        kind === "blank" ? t("notes.untitled") : t(`notes.kind_${kind}`),
        body,
      );
      await refresh();
      setOpenId(id);
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
          <div className="relative flex items-center gap-1">
            {/* The common action stays one click. Putting the shapes *behind* the New button made
                every blank note cost a menu, which is the wrong trade: most notes are blank, and
                the release check that drives a first install caught it immediately. */}
            <Button size="sm" onClick={() => void create("blank")}>
              {t("notes.new")}
            </Button>
            <button
              type="button"
              aria-label={t("notes.kind")}
              aria-expanded={picking}
              onClick={() => setPicking((p) => !p)}
              className="border-line text-fg-dim hover:text-fg h-8 rounded-[var(--radius-card)] border px-1.5 text-xs"
            >
              ▾
            </button>
            {picking && (
              <ul
                className="border-line bg-bg-raised absolute end-0 z-10 mt-1 w-40 rounded-[var(--radius-card)] border py-1 shadow-[var(--shadow-pop)]"
                aria-label={t("notes.kind")}
                data-testid="note-kinds"
              >
                {KINDS.map((kind) => (
                  <li key={kind}>
                    <button
                      type="button"
                      onClick={() => void create(kind)}
                      className="hover:bg-bg-soft w-full px-3 py-1.5 text-start text-sm"
                    >
                      {t(`notes.kind_${kind}`)}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          {notes.length === 0 && (
            <p className="text-fg-faint text-meta px-2 py-6">{t("notes.empty")}</p>
          )}
          {grouped.map(([day, entries]) => (
            <section key={day} className="mb-3">
              <p className="text-fg-faint text-micro px-2 pb-1 font-semibold tracking-wider uppercase">
                {day}
              </p>
              <ul className="space-y-0.5">
                {entries.map((note) => (
                  <li key={note.id}>
                    <button
                      type="button"
                      onClick={() => setOpenId(note.id)}
                      aria-current={note.id === openId}
                      className={`flex w-full items-center gap-1.5 rounded-lg px-2 py-1.5 text-left text-sm ${
                        note.id === openId
                          ? "bg-accent-soft text-accent"
                          : "text-fg-dim hover:bg-bg-soft hover:text-fg"
                      }`}
                    >
                      {/* The same mark the library draws. A colour is meant to identify a note
                          wherever it appears, and this list was the one place it did not. */}
                      {note.color && <Dot colour={note.color} />}
                      <span className="truncate">{note.title}</span>
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
            className="border-danger/30 bg-danger-soft text-danger text-meta border-b px-4 py-2"
          >
            {error}
          </p>
        )}

        {openId === null ? (
          // Centred in the pane rather than pinned a fixed distance from the top: `mt-24` puts a
          // grey sentence in the upper third of a tall empty column, which is what the whole
          // interface used to look like.
          // With a way out. An empty state that only describes the emptiness is a dead end, and
          // this one is the largest surface on the screen for anybody who has not written a note
          // yet — the button they need is a 288px column away in the corner of another pane.
          <Empty
            full
            icon={NotebookPen}
            title={t("notes.pick")}
            action={
              <Button size="sm" variant="secondary" onClick={() => void create("blank")}>
                {t("notes.new")}
              </Button>
            }
          />
        ) : (
          <NoteEditor
            // Keyed, so switching notes remounts rather than reconciling: the editor holds a
            // debounce timer and an unsaved buffer, and carrying either into a different document
            // is how one note's sentence ends up in another.
            key={openId}
            id={openId}
            onSaved={() => void refresh()}
            onRemoved={() => {
              setOpenId(null);
              void refresh();
            }}
          />
        )}
      </section>
    </div>
  );
}
