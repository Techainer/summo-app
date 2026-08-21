import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Comments } from "../components/meeting/Comments";
import { LiveMeeting } from "../components/meeting/LiveMeeting";
import { Markdown } from "../components/page/Markdown";
import { NoteEditor } from "../components/page/NoteEditor";
import { cn } from "../lib/cn";
import { useErrorText } from "../lib/errors";
import { useI18n } from "../i18n/context";
import { DraftPanel } from "../components/meeting/DraftPanel";
import { AskPanel } from "../components/meeting/Ask";
import { Export } from "../components/meeting/Export";
import { Player, type PlayerHandle } from "../components/meeting/Player";
import { TranscriptChips } from "../components/meeting/TranscriptChips";
import { Button, Card, CardBody, CardHeader, SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";
import { DraftClient, templates, type Draft } from "../lib/draft";
import { url, type MeetingDetail } from "../lib/library";
import { NOTES_HEADING, NoteClient } from "../lib/notes";
import { TaskClient } from "../lib/tasks";
import { formatDuration } from "../lib/duration";
import type { Naming } from "../components/meeting/TranscriptChips";
import { useLoad } from "../lib/use-load";

type Pane = "comments" | "transcript";

// Keys, resolved at render — a module-level array is built before any provider exists.
//
// Transcript first, and first is also the default. What somebody opens a recorded meeting to do is
// read what was said; comments are what they come back for later, once there is a conversation to
// have about it. Defaulting to comments meant the widest panel on the screen opened on "nobody has
// said anything yet" every time — visible in the screenshot the landing page was built from, where
// two fifths of the window is an empty thread.
const PANES = [
  { value: "transcript" as const, labelKey: "meeting.transcript" },
  { value: "comments" as const, labelKey: "comments.title" },
];

/**
 * One page: what was said, what it meant, and what to do about it — or, if nobody said anything,
 * what somebody typed.
 *
 * There used to be two screens here. A recording opened at `/meetings/<id>` and a typed note opened
 * at `/notes?open=<id>`, which meant every part of the app that can point at a document had to know
 * which kind it was pointing at, and five of them got it wrong in the same way: the search palette,
 * both citation lists, the home screen's recent strip and the record screen all sent a *note* to
 * `/notes` **with nothing open**. Each carried a comment apologising for it. Clicking a search
 * result for a note, or the citation under an answer that quoted one, opened a screen and lost the
 * document — for the one control whose entire job is letting a reader check a claim.
 *
 * The vault has never had two kinds of document. A note is a meeting with an empty transcript;
 * `summo_vault::note` is a hundred lines of filing on top of the same parser. This is that model
 * with one address: `/pages/<id>` opens whatever is there, and nothing that links to a document has
 * to ask what it is first.
 *
 * The two bodies stay different, because they are. A recording is read — a player, a transcript,
 * threads pinned to utterances. A note is written, and what it needs is a textarea and to be left
 * alone. What they share is the address, the header and the way they are filed.
 */
export function PageScreen() {
  const { t, locale } = useI18n();
  const say = useErrorText();
  const navigate = useNavigate();
  const { pageId } = useParams({ from: "/pages/$pageId" });
  const engine = useEngine();
  const { library, handshake, people } = engine;
  // Stored with the page it belongs to, so switching pages does not need an effect to blank it
  // first. The `setDetail(null)` that used to do that ran synchronously inside the effect — one
  // extra render per navigation, and a frame in which the new page's title sat above the old
  // page's transcript.
  const [loaded, setLoaded] = useState<{ id: string; detail: MeetingDetail } | null>(null);
  const detail = loaded?.id === pageId ? loaded.detail : null;
  const [error, setError] = useState<string | null>(null);
  const [pane, setPane] = useState<Pane>("transcript");
  const [at, setAt] = useState(0);
  const [summarising, setSummarising] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const player = useRef<PlayerHandle>(null);
  const drafts = useMemo(() => new DraftClient(handshake), [handshake]);
  const tasks = useMemo(() => new TaskClient(handshake), [handshake]);
  const notes = useMemo(() => new NoteClient(handshake), [handshake]);
  const [naming, setNaming] = useState<string | null>(null);
  /** Ticks in flight, and what the user asked for — see `pending` on {@link Markdown}. */
  const [ticking, setTicking] = useState<ReadonlyMap<string, boolean>>(new Map());

  const resolveImage = useCallback((link: string) => notes.src(link), [notes]);

  /**
   * Tick a checkbox in the body of this page.
   *
   * Through the task API by id, which is the same call the board makes, so the two cannot disagree
   * about what a tick means — and the page is reloaded afterwards because the daemon rewrote the
   * file this screen is drawing.
   */
  const tick = useCallback(
    (id: string, done: boolean) => {
      setTicking((busy) => new Map(busy).set(id, done));
      void (async () => {
        try {
          await tasks.move(id, { status: done ? "done" : "todo" });
          setLoaded({ id: pageId, detail: await library.detail(pageId) });
        } catch (e) {
          setError(say(e));
        } finally {
          // After the reload, so the box is never drawn from the old file for a frame — which is
          // the flicker this whole mechanism exists to avoid.
          setTicking((busy) => {
            const next = new Map(busy);
            next.delete(id);
            return next;
          });
        }
      })();
    },
    [library, pageId, say, tasks],
  );

  // One place to apply whatever the draft endpoints return, so the note and the panel cannot drift.
  const applyDraft = useCallback(
    async (next: Draft | null) => {
      setDraft(next);
      setLoaded({ id: pageId, detail: await library.detail(pageId) });
      setError(null);
    },
    [library, pageId],
  );

  const run = useCallback(
    async (work: () => Promise<Draft | null>) => {
      setSummarising(true);
      try {
        await applyDraft(await work());
      } catch (e) {
        setError(say(e));
      } finally {
        setSummarising(false);
      }
    },
    [applyDraft, say],
  );

  // The daemon reports file names (`mic.opus`); the route takes lane names. Strip the extension
  // here rather than teaching the player about container formats.
  const lanes = useMemo(
    () =>
      (detail?.audio ?? []).map((file) => {
        const key = file.replace(/\.[^.]+$/, "");
        return {
          key,
          label: key === "mic" ? t("record.microphone") : t("record.system"),
          url: url(handshake, `/meetings/${encodeURIComponent(pageId)}/audio/${key}`),
        };
      }),
    [detail?.audio, handshake, pageId, t],
  );

  const marks = useMemo(
    () => (detail?.transcript ?? []).map((segment) => segment.t0),
    [detail?.transcript],
  );

  /**
   * Which summary shape to ask for, empty for "let the meeting decide".
   *
   * The list is read whenever a meeting has no summary yet, which is the only moment it can be
   * acted on. Its failure is silent: a picker that could not be fetched is one fewer option on a
   * button that still works.
   */
  const [template, setShape] = useState("");
  const shapes = useLoad(
    useCallback(
      async () => (detail && detail.sections.length === 0 ? await templates(handshake) : []),
      [detail, handshake],
    ),
    [detail, handshake],
  );
  const summarise = () => void run(() => drafts.generate(pageId, template || undefined));

  /**
   * Who is still a label rather than a person, and everybody they could be.
   *
   * Read beside the detail rather than inside the transcript, because naming one voice *rewrites*
   * the transcript — so the answer and the thing it changes have to be reloaded together, or the
   * chips go on showing `S2` beside somebody who now has a name.
   *
   * Its failure is deliberately not shown. This is an offer on top of a transcript already on
   * screen; a red bar because the suggestions could not be fetched would be a failure report about
   * something the reader never asked for.
   */
  /**
   * The page this one is inside, when it is inside one.
   *
   * A second request rather than a field on the first: the daemon hands back a parent's *id*,
   * because that is what the file says, and a title is the parent's business and changes when the
   * parent is renamed. Its failure is silent — a missing breadcrumb is a missing convenience, and a
   * red bar about a page the reader did not ask for would be a failure report about nothing.
   */
  const parentOf = loaded?.id === pageId ? (loaded.detail.summary.parent ?? null) : null;
  const container = useLoad(
    useCallback(
      async () => (parentOf ? await library.detail(parentOf) : null),
      [library, parentOf],
    ),
    [library, parentOf],
  );

  const asked = useLoad(
    useCallback(async () => {
      const [unnamed, view] = await Promise.all([people.unknowns(pageId), people.list()]);
      return { unnamed, book: view.people };
    }, [people, pageId]),
    [people, pageId],
  );
  const reask = asked.reload;

  const name = useCallback(
    (label: string, who: string) => {
      if (!who.trim()) return;
      setNaming(label);
      void (async () => {
        try {
          await people.nameVoice(pageId, label, who.trim());
          // Both, in this order: the transcript has been rewritten under us, and the question has
          // been answered.
          setLoaded({ id: pageId, detail: await library.detail(pageId) });
          reask();
        } catch (e) {
          setError(say(e));
        } finally {
          setNaming(null);
        }
      })();
    },
    [library, pageId, people, reask, say],
  );

  const speakers: Naming = useMemo(
    () => ({
      unnamed: Object.fromEntries((asked.data?.unnamed ?? []).map((voice) => [voice.label, voice])),
      people: asked.data?.book ?? [],
      busy: naming,
      onName: name,
    }),
    [asked.data, naming, name],
  );

  useEffect(() => {
    let cancelled = false;
    library
      .detail(pageId)
      .then((d) => !cancelled && setLoaded({ id: pageId, detail: d }))
      .catch((e: unknown) => {
        if (!cancelled) setError(say(e));
      });
    // A meeting may already have a summary nobody has agreed to.
    drafts
      .get(pageId)
      .then((d) => !cancelled && setDraft(d))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // `recording` is a dependency: the daemon rewrites this document as the meeting ends — it
    // writes the duration, finishes the transcript, and the index stops calling it a note. Without
    // this, the page kept whatever it loaded *during* the recording, which is why the first thing a
    // user saw after their first meeting was the plain editor with the transcript's own bookkeeping
    // in it: `**[00:00:00] me** — … <!-- seq:0 end:3.40 -->`.
  }, [library, drafts, pageId, say, t, engine.session.recording]);

  /**
   * And once more, a moment later.
   *
   * Stopping is not instant on the daemon's side: the socket says the session ended, then the
   * recorder flushes the last utterance, writes the duration and lets the index re-read the file.
   * A single fetch on the way down wins that race about half the time — and losing it means the
   * meeting renders as an empty note, which is what it looked like the first time this was fixed.
   */
  const wasRecording = useRef(engine.session.recording);
  useEffect(() => {
    const stopped = wasRecording.current && !engine.session.recording;
    wasRecording.current = engine.session.recording;
    if (!stopped) return undefined;

    const timers = [1200, 3000].map((after) =>
      window.setTimeout(() => {
        library
          .detail(pageId)
          .then((d) => setLoaded({ id: pageId, detail: d }))
          .catch(() => undefined);
      }, after),
    );
    return () => timers.forEach((timer) => window.clearTimeout(timer));
  }, [engine.session.recording, library, pageId]);

  // Only when there is nothing else to show. Every failure on this screen used to replace it — so
  // asking the agent to reword a sentence while the model was unreachable left the reader with one
  // red line where their meeting had been: the transcript, the summary, the comments and the player
  // all gone, over an optional request they could simply not have made. A failure that arrives on
  // top of a page that loaded belongs on top of it, and is drawn beside the content below.
  if (error && !detail) {
    return (
      <div className="p-5">
        <p
          role="alert"
          className="border-danger/30 bg-danger-soft text-danger text-meta rounded-lg border px-3 py-2"
        >
          {error}
        </p>
      </div>
    );
  }

  if (!detail) {
    return (
      <p className="text-fg-faint grid h-full place-items-center text-center">
        {t("meeting.opening")}
      </p>
    );
  }

  const { summary, sections, transcript } = detail;
  // A document with a transcript in it is a meeting, whatever the index decided.
  //
  // The index reads a window at the top of the file and looks for `## Transcript`; a recording that
  // has only just stopped can miss it — no duration written yet, the heading pushed past the
  // window by notes typed during the meeting — and the page then opened in the plain editor, where
  // the transcript's own bookkeeping (`<!-- seq:0 end:3.43 -->`) is text like any other. A user saw
  // that at the end of their first recording.
  //
  // `transcript`, not the sections: the parser lifts the transcript out of the document into its
  // own field, so a meeting's sections are only the prose around it.
  const note = summary.kind === "note" && transcript.length === 0;

  /**
   * What the user typed, kept apart from what the model wrote.
   *
   * Both are `##` sections in the same file, so the page drew them as one run of identical cards in
   * whatever order the file happened to hold — a person's own notes could land under the summary
   * of them, under the export panel, anywhere. They are not peers: one is the reason to trust the
   * other. Splitting them here is what lets the notes lead the column and the summary say, in its
   * own colour, that a model wrote it.
   */
  const typed = sections.find((s) => s.heading === NOTES_HEADING);
  /**
   * The summary, as it stands on disk.
   *
   * Draft sections are excluded because {@link DraftPanel} owns those until they are approved, and
   * drawing them in both places shows the same paragraph twice.
   */
  const summarised = sections.filter((s) => !s.draft && s.heading !== NOTES_HEADING);

  /**
   * This page, right now, with the microphone open.
   *
   * The document is being written by the daemon's recorder while this renders, so the screen shows
   * the two things that are moving — what the user types and what is being heard — and none of the
   * things that only make sense afterwards: a player with nothing to play, an export of a meeting
   * that has not finished, a summary of a conversation still happening.
   */
  const live = engine.session.recording && engine.session.meeting === pageId;

  const meta = (
    <>
      {/* Where this page is, when it is inside another one. A sub-page opened from a search result
          or a citation arrives with no context at all otherwise — it looks like a top-level note
          that happens to be called "Ngân sách", and the thing it is part of is invisible. */}
      {summary.parent ? (
        <Link
          to="/pages/$pageId"
          params={{ pageId: summary.parent }}
          className="text-fg-dim hover:text-fg text-meta"
        >
          ← {container.data?.summary.title ?? t("meeting.back")}
        </Link>
      ) : (
        <Link to="/library" search={{}} className="text-fg-dim hover:text-fg text-meta">
          ← {t("meeting.back")}
        </Link>
      )}
      {/* A recording's title is a heading; a note's title is the first line of the note, and the
          editor below already shows it. Drawing it twice would put the user's own first sentence
          above the field they are about to edit it in. */}
      {!note && <h1 className="mt-2 text-2xl font-semibold tracking-tight">{summary.title}</h1>}
      <div className="text-fg-dim text-meta mt-2 flex flex-wrap items-center gap-2">
        <Pill>{summary.day}</Pill>
        {!note && <Pill>{formatDuration(summary.duration, locale)}</Pill>}
        {summary.folder && <Pill>{summary.folder}</Pill>}
        {summary.tags.map((tag) => (
          <Pill key={tag}>#{tag}</Pill>
        ))}
      </div>
    </>
  );

  if (live) {
    return (
      <div className="flex h-full min-h-0 flex-col gap-4 p-5">
        {meta}
        <LiveMeeting
          initialNotes={detail?.sections.find((s) => s.heading === NOTES_HEADING)?.body ?? ""}
        />
      </div>
    );
  }

  if (note) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="px-5 pt-5 pb-3">{meta}</div>
        <NoteEditor
          key={pageId}
          id={pageId}
          className="border-line border-t"
          onRemoved={() => void navigate({ to: "/library", search: {} })}
          onOpenPage={(id) => void navigate({ to: "/pages/$pageId", params: { pageId: id } })}
        />
      </div>
    );
  }

  return (
    <div className="p-5">
      {meta}

      {error && (
        <p
          role="alert"
          className="border-danger/30 bg-danger-soft text-danger text-meta mt-3 flex items-start justify-between gap-3 rounded-lg border px-3 py-2"
        >
          <span>{error}</span>
          <button
            type="button"
            onClick={() => setError(null)}
            className="shrink-0 font-medium hover:underline"
          >
            {t("common.dismiss")}
          </button>
        </p>
      )}

      <div className="mt-5 grid gap-4 lg:grid-cols-[1fr_380px]">
        {/* The order is the order somebody works in, which is not the order this column was built
            in. It ran player, summary machinery, ask, export, and then — last, under the button
            that mails it — whatever the person had actually written.

            Now: what you wrote, what it was summarised into, the recording it came from, and then
            the two things you do with it. Reading beats listening back on almost every visit, so
            the transport moved below the words; export moved under the summary because you cannot
            sensibly send a thing you have not read. */}
        <div className="space-y-4">
          {typed && typed.body.trim() && (
            <Card>
              <CardHeader title={t("meeting.your_notes")} />
              <CardBody>
                <Markdown
                  markdown={typed.body}
                  resolveImage={resolveImage}
                  onToggleTask={tick}
                  pending={ticking}
                />
              </CardBody>
            </Card>
          )}

          {draft && (
            <DraftPanel
              draft={draft}
              busy={summarising}
              onRefine={(heading, selection, instruction) =>
                void run(() => drafts.refine(pageId, heading, selection, instruction))
              }
              onChat={(message) => void run(() => drafts.chat(pageId, message))}
              onConfirm={() =>
                void run(async () => {
                  await drafts.confirm(pageId);
                  return null;
                })
              }
              onDiscard={() =>
                void run(async () => {
                  await drafts.discard(pageId);
                  return null;
                })
              }
            />
          )}

          {!draft && summarised.length === 0 ? (
            <Card>
              {/* A column, because these are three separate things stacked and they were laid out
                  as flowing text: an inline link followed by an inline-flex button put the settings
                  link and the summarise button on the same line, overlapping, on every meeting with
                  no summary yet. */}
              <CardBody className="flex flex-col items-center gap-3 pt-4 text-center">
                <p className="text-fg-faint">{t("meeting.no_summary_yet")}</p>
                <Button variant="primary" busy={summarising} onClick={() => void summarise()}>
                  {t("meeting.summarise_now")}
                </Button>
                {/* Which shape. Four templates ship and `generate` has always taken one; nothing
                    ever passed it, so a one-to-one and a sprint review were both written up as a
                    weekly meeting unless the file's tags happened to match. Absent means "let the
                    tags and the title choose", which is the old behaviour and stays the default. */}
                {shapes.data && shapes.data.length > 1 && (
                  <div className="flex flex-wrap items-center justify-center gap-1.5">
                    <span className="text-fg-faint text-micro">{t("meeting.summary_shape")}</span>
                    {[{ id: "", name: t("meeting.shape_auto") }, ...shapes.data].map((shape) => (
                      <button
                        key={shape.id}
                        type="button"
                        onClick={() => setShape(shape.id)}
                        aria-pressed={shape.id === template}
                        className={cn(
                          "text-micro rounded-[var(--radius-pill)] border px-2.5 py-1 transition-colors",
                          shape.id === template
                            ? "border-accent bg-accent-soft text-accent"
                            : "border-line hover:border-fg-faint",
                        )}
                      >
                        {shape.name}
                      </button>
                    ))}
                  </div>
                )}
                {/* Where the summariser is configured, under the action rather than above it: this
                    is what somebody reaches for when the button did not give them what they
                    expected, and "Trí tuệ" was a settings section nobody looking for a language
                    model ever found. */}
                <button
                  type="button"
                  onClick={() => void navigate({ to: "/settings", search: { section: "ai" } })}
                  className="text-fg-faint text-micro underline"
                >
                  {t("meeting.open_ai_settings")}
                </button>
              </CardBody>
            </Card>
          ) : (
            summarised.map((section) => (
              <Card key={section.heading}>
                <CardHeader title={section.heading} />
                <CardBody>
                  {/* The body as it is on disk, comments and all: the renderer strips them, and
                      the ids inside them are what makes a checkbox here tickable. */}
                  <Markdown
                    markdown={section.body}
                    resolveImage={resolveImage}
                    onToggleTask={tick}
                    pending={ticking}
                  />
                </CardBody>
              </Card>
            ))
          )}

          {/* Below the words. Listening back is the rarer visit — the transcript is searchable and
              the summary is already written — and a transport at the top of the column put the one
              control nobody uses where the eye lands first. */}
          <Player lanes={lanes} marks={marks} onTime={setAt} ref={player} />

          {/* Asked *about* what the meeting concluded, so it sits under the conclusion. Writing the
              follow-up email is not a feature of its own — it is one of the things people ask for,
              and what comes back is a note like any other. */}
          <AskPanel meeting={pageId} />

          {/* Taking it out again, last: read what happened, then send it to somebody. */}
          <Export meeting={pageId} title={summary.title} day={summary.day ?? ""} recorded={!note} />
        </div>

        <aside className="space-y-3">
          <SegmentedControl
            label={t("meeting.right_panel")}
            options={PANES.map((p) => ({
              value: p.value,
              label: t(p.labelKey),
            }))}
            value={pane}
            onChange={setPane}
            size="sm"
          />
          <Card className="h-[62vh] overflow-hidden">
            {pane === "transcript" ? (
              <TranscriptChips
                segments={transcript}
                at={at}
                onSeek={(seconds) => player.current?.seek(seconds)}
                reading
                naming={speakers}
              />
            ) : (
              <CardBody className="flex h-full min-h-0 flex-col pt-4">
                <Comments
                  meeting={pageId}
                  // A comment is pinned to an utterance; the player seeks in seconds. The
                  // transcript is what knows the difference, and it is already here.
                  onSeek={(seq) => {
                    const found = transcript.find((segment) => segment.seq === seq);
                    if (found) player.current?.seek(found.t0);
                  }}
                />
              </CardBody>
            )}
          </Card>
        </aside>
      </div>
    </div>
  );
}

function Pill({ children }: { children: React.ReactNode }) {
  return <span className="border-line bg-bg-soft rounded-full border px-2.5 py-1">{children}</span>;
}
