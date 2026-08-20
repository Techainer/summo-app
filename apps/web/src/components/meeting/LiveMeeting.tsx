import { Suspense, lazy, useEffect, useRef, useState } from "react";

import { Square } from "lucide-react";

import { Transcript } from "../Transcript";
import { Button, SectionTitle } from "../ui";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { SAVE_DEBOUNCE_MS } from "../../lib/notes";

/**
 * The meeting while it is happening: what you type, beside what was said.
 *
 * A meeting is a note with a transcript in it. That has been true of the file since the vault was
 * written — `summo_vault::note` is filing on top of the same parser — and it was not true of the
 * screen: recording had a page of its own showing a transcript and offering nowhere to write, so
 * the one thing a person does in a meeting was the one thing the app could not hold.
 *
 * Two columns, and the left one is the point. What somebody types during a meeting is a decision
 * being recorded by the person who made it; the transcript is a machine's account of the noise in
 * the room. The summariser is told the same thing — see `NOTES_RULES` in `summo-llm` — so a note
 * saying Thursday beats a transcript that heard Wednesday.
 *
 * Deliberately *not* `NoteEditor`. That one reads and writes through `/notes/{id}`, and this
 * document is being rewritten by the daemon's recorder every ten seconds: two writers, one file,
 * and the loser is whoever typed. Here the text goes over the session socket, into the recorder
 * that owns the document.
 */
const RichNote = lazy(() => import("../page/RichNote").then((m) => ({ default: m.RichNote })));

export function LiveMeeting({ initialNotes = "" }: { initialNotes?: string }) {
  const { transcript, notes, stop } = useEngine();
  const t = useT();

  const [text, setText] = useState(initialNotes);
  const [plain, setPlain] = useState(false);
  const latest = useRef(text);
  const timer = useRef<number | null>(null);

  useEffect(() => {
    latest.current = text;
  }, [text]);

  // The same debounce a note uses, and the same reason: a keystroke is not a save. What is
  // different is where it goes — the recorder writes the file the moment this lands, so there is no
  // second timer on the other side.
  const edit = (next: string) => {
    setText(next);
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => notes(latest.current), SAVE_DEBOUNCE_MS);
  };

  // Leaving mid-sentence saves, exactly as the note editor does. Stopping a recording unmounts
  // this, and the last thing typed before pressing stop is usually the thing that mattered.
  useEffect(
    () => () => {
      if (timer.current === null) return;
      window.clearTimeout(timer.current);
      timer.current = null;
      notes(latest.current);
    },
    [notes],
  );

  return (
    <div className="grid min-h-0 flex-1 gap-4 lg:grid-cols-2">
      <section className="flex min-h-0 flex-col gap-2.5">
        <SectionTitle>{t("meeting.your_notes")}</SectionTitle>
        <div className="border-line bg-bg-raised rounded-card min-h-0 flex-1 overflow-y-auto border shadow-[var(--shadow-card)]">
          {plain ? (
            // The same fallback a note has: an editor that cannot hold something without changing
            // it hands the text back as text rather than quietly rewriting it.
            <textarea
              aria-label={t("meeting.your_notes")}
              value={text}
              onChange={(e) => edit(e.target.value)}
              className="h-full w-full resize-none bg-transparent p-4 font-mono text-[13px] outline-none"
            />
          ) : (
            <Suspense fallback={null}>
              <RichNote
                markdown={text}
                onChange={edit}
                onUnsupported={() => setPlain(true)}
                className="p-4"
              />
            </Suspense>
          )}
        </div>
      </section>

      <section className="flex min-h-0 flex-col gap-2.5">
        <div className="flex items-center justify-between gap-3">
          <SectionTitle>{t("meeting.transcript")}</SectionTitle>
          {/* Stopping, spelled out. The only way to end a meeting was the red pill in the header
              that shows the elapsed time — a control somebody has to guess at, on the one screen
              where they know exactly what they want to do next. */}
          <Button size="sm" variant="danger" onClick={stop}>
            <Square aria-hidden="true" className="me-1.5 size-3" />
            {t("record.stop")}
          </Button>
        </div>
        <div className="min-h-0 flex-1">
          <Transcript segments={transcript.segments} />
        </div>
      </section>
    </div>
  );
}
