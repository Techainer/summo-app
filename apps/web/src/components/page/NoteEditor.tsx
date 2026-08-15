import { AnimatePresence, m } from "motion/react";
import { Suspense, lazy, useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "../ui";

import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { useT } from "../../i18n/context";
import { NoteClient, SAVE_DEBOUNCE_MS, titleFrom } from "../../lib/notes";
import { cn } from "../../lib/cn";

/**
 * Writing in a note.
 *
 * Extracted from the notes screen so that opening a note from the tree, from a search result or
 * from a citation lands in the same editor rather than in a second one. Two textareas with two
 * copies of the autosave would have drifted the first time either was touched, and the drift would
 * have been silent: both would still look like a note.
 *
 * Three decisions that shape how it feels, all inherited from where this came from.
 *
 * **There is no Save button.** A note is a file, and a file that only exists once you remember to
 * press something is a file people lose. It autosaves two seconds after you stop typing, and says
 * so quietly rather than flashing a toast at every pause.
 *
 * **The first line is the title.** Asking for a title in its own field before you may start typing
 * is the step that makes people close a note app and open a text editor. What you type becomes the
 * heading; the file keeps the name it was born with.
 *
 * **The whole thing is one textarea, not a rich editor.** The vault is Markdown that a user opens
 * in Obsidian, greps and backs up; an editor that stored its own shape would break that promise the
 * first time it round-tripped something it did not understand. Tasks work here because
 * `- [ ] @ngoc …` is parsed from the file, not from a widget.
 */
/**
 * Loaded when a note is opened, not when the app is.
 *
 * ProseMirror and its extensions are 128 kB gzipped — more than a third of everything else here —
 * and most of what this app does is recording, reading and searching. Paying for the editor on the
 * home screen would make the whole thing slower to start for the sake of a screen the user may not
 * open. It is a local server, but it is also a phone browser on a bad connection.
 */
const RichNote = lazy(() => import("./RichNote").then((m) => ({ default: m.RichNote })));

export function NoteEditor({
  id,
  onSaved,
  onRemoved,
  onOpenPage,
  className,
}: {
  id: string;
  /** The title may have changed, so whoever is listing notes should re-read them. */
  onSaved?: () => void;
  onRemoved?: () => void;
  /**
   * Open a page this one now contains.
   *
   * Given by whoever is showing the tree, because making a sub-page is only half the gesture: the
   * other half is that the person wanted to write in it. Absent where there is nowhere to go.
   */
  onOpenPage?: (id: string) => void;
  className?: string;
}) {
  const { handshake } = useEngine();
  const say = useErrorText();
  const t = useT();
  const client = useMemo(() => new NoteClient(handshake), [handshake]);

  const [text, setText] = useState("");
  const [saved, setSaved] = useState(true);
  const [error, setError] = useState<string | null>(null);
  /**
   * Which editor this note gets.
   *
   * `null` until the note has been read — a rich editor mounted on an empty string would report
   * "this is fine" about a document that has not arrived. `rich` is the default afterwards, and
   * `plain` is where a note goes when the editor cannot hold it without changing it: see
   * [`RichNote`]. Falling back is the guarantee, not a failure — a note with a table opens as text
   * and keeps its table.
   */
  const [mode, setMode] = useState<"rich" | "plain" | null>(null);

  // Held in a ref as well as in state: the debounce fires from a timer that closed over an older
  // render, and saving the text from two seconds ago would undo the last two seconds of typing.
  const latest = useRef(text);
  // In an effect rather than during render: mutating a ref while rendering is what React forbids,
  // and it buys nothing here — the timer that reads this fires later than any effect.
  useEffect(() => {
    latest.current = text;
  }, [text]);
  const timer = useRef<number | null>(null);

  // The note being edited, so a save that lands after the user has moved on cannot write one note's
  // text into another. Switching notes is one keystroke in the tree and the read is a round trip.
  //
  // Both callers key this component on the note id, so in practice `id` never changes under a
  // mounted editor — carrying a debounce timer and an unsaved buffer into a different document is
  // how one note's sentence ends up in another. This is the belt to that pair of braces.
  const showing = useRef(id);

  useEffect(() => {
    let cancelled = false;
    showing.current = id;
    client
      .read(id)
      .then((note) => {
        if (cancelled) return;
        // The title is the first line, so it is shown as the first line — the file stores them
        // apart, and stitching them back together is what makes the editor feel like one document.
        // `text`, not `body`: the latter stops at the first heading.
        const body = note.text.trim();
        setText(body ? `${note.title}\n\n${body}` : note.title);
        setMode("rich");
      })
      .catch((e: unknown) => !cancelled && setError(say(e)));
    return () => {
      cancelled = true;
    };
  }, [client, id, say]);

  const persist = useCallback(async () => {
    const writing = showing.current;
    const { title, rest } = titleFrom(latest.current);
    try {
      await client.save(writing, rest, title);
      // Only if the editor is still showing what was written. Otherwise the tick belongs to a note
      // the user has already left, and would claim their current unsaved text was safe.
      if (showing.current === writing) setSaved(true);
      onSaved?.();
    } catch (e) {
      // Left as unsaved on purpose: telling the user it saved when it did not is how a note is
      // lost quietly rather than loudly.
      setError(say(e));
    }
  }, [client, onSaved, say]);

  // The title line and everything under it. Split here rather than held as two states, because the
  // file is one document and the debounce, the save and the unsaved flag are all about the whole
  // of it — two states would need two of each and a rule about which wins.
  const { title, rest } = split(text);

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

  const remove = async () => {
    try {
      await client.remove(id);
      onRemoved?.();
    } catch (e) {
      setError(say(e));
    }
  };

  /**
   * A page inside this one.
   *
   * Made before it is linked, and linked with the id it came back with, so the link in the file
   * always points at something that exists. Doing it the other way round — write the link, create
   * the page — leaves a dangling link in the note every time the daemon says no.
   *
   * Named `t("notes.untitled")` rather than asking. A sub-page is created mid-sentence; a dialog
   * demanding a title first is the step that makes people stop making them.
   */
  const subpage = useCallback(async () => {
    try {
      const title = t("notes.untitled");
      const { id: child } = await client.create(title, "", id);
      onSaved?.();
      return { id: child, title };
    } catch (e) {
      setError(say(e));
      return null;
    }
  }, [client, id, onSaved, say, t]);

  const upload = useCallback((file: File) => client.upload(file), [client]);
  const resolve = useCallback((link: string) => client.src(link), [client]);

  return (
    <div className={cn("flex min-h-0 flex-1 flex-col", className)}>
      {error && (
        <p
          role="alert"
          className="border-danger/30 bg-danger-soft text-danger text-meta border-b px-4 py-2"
        >
          {error}
        </p>
      )}

      <div className="border-line flex items-center gap-3 border-b px-4 py-2">
        <AnimatePresence mode="wait">
          <m.span
            key={saved ? "saved" : "editing"}
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            className="text-fg-faint text-micro"
          >
            {saved ? t("notes.saved") : t("notes.saving")}
          </m.span>
        </AnimatePresence>
        {/* Why this note looks different from the last one. An editor that silently degrades is one
            the user assumes is broken; saying so once, quietly, is the difference between a rule
            and a glitch. */}
        {mode === "plain" && (
          <span className="text-fg-faint text-micro truncate" title={t("notes.plain_mode")}>
            {t("notes.plain_mode")}
          </span>
        )}
        <span className="flex-1" />
        <Button size="sm" variant="ghost" onClick={() => void remove()}>
          {t("common.delete")}
        </Button>
      </div>

      {/* The title is the first line and stays a plain input, in both modes. It is one line of
          text that names a file; formatting it would be formatting a filename. */}
      <input
        value={title}
        onChange={(e) => edit(`${e.target.value}\n${rest}`)}
        onBlur={() => void persist()}
        aria-label={t("notes.title_field")}
        spellCheck={false}
        placeholder={t("notes.untitled")}
        className="font-reading text-fg placeholder:text-fg-faint bg-transparent px-6 pt-5 pb-1 text-2xl font-semibold tracking-tight outline-none"
      />

      {mode === "rich" ? (
        // No fallback element: the chunk lands in a frame or two on a local daemon, and a spinner
        // that flashes is worse than a pane that is briefly blank.
        <Suspense fallback={null}>
          <RichNote
            // Keyed on the note, so a different document is a different editor rather than one
            // editor asked to change its mind about what it is holding.
            key={id}
            markdown={rest}
            onChange={(next) => edit(`${title}\n\n${next}`)}
            onUnsupported={() => setMode("plain")}
            onUpload={upload}
            resolveImage={resolve}
            onNewSubpage={subpage}
            onOpenPage={onOpenPage}
          />
        </Suspense>
      ) : (
        <textarea
          value={rest}
          onChange={(e) => edit(`${title}\n\n${e.target.value}`)}
          onBlur={() => void persist()}
          aria-label={t("notes.body")}
          spellCheck={false}
          placeholder={t("notes.placeholder")}
          // `font-reading` and a measure: a note is prose, and prose at full window width is prose
          // nobody re-reads.
          className="font-reading text-body min-h-0 flex-1 resize-none bg-transparent px-6 pb-5 leading-relaxed outline-none"
        />
      )}
    </div>
  );
}

/** The first line, and everything after it. */
function split(text: string): { title: string; rest: string } {
  const at = text.indexOf("\n");
  if (at === -1) return { title: text, rest: "" };
  return { title: text.slice(0, at), rest: text.slice(at + 1).replace(/^\n+/, "") };
}
