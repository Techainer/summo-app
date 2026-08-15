import { Link, useNavigate, useParams } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Comments } from "../components/meeting/Comments";
import { NoteEditor } from "../components/page/NoteEditor";
import { useErrorText } from "../lib/errors";
import { useI18n } from "../i18n/context";
import { DraftPanel } from "../components/meeting/DraftPanel";
import { AskPanel } from "../components/meeting/Ask";
import { Player, type PlayerHandle } from "../components/meeting/Player";
import { TranscriptChips } from "../components/meeting/TranscriptChips";
import { Button, Card, CardBody, CardHeader, SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";
import { DraftClient, readable, type Draft } from "../lib/draft";
import { url, type MeetingDetail } from "../lib/library";
import { formatDuration } from "../lib/duration";

type Pane = "comments" | "transcript";

// Keys, resolved at render — a module-level array is built before any provider exists.
const PANES = [
  { value: "comments" as const, labelKey: "comments.title" },
  { value: "transcript" as const, labelKey: "meeting.transcript" },
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
  const { library, handshake } = useEngine();
  // Stored with the page it belongs to, so switching pages does not need an effect to blank it
  // first. The `setDetail(null)` that used to do that ran synchronously inside the effect — one
  // extra render per navigation, and a frame in which the new page's title sat above the old
  // page's transcript.
  const [loaded, setLoaded] = useState<{ id: string; detail: MeetingDetail } | null>(null);
  const detail = loaded?.id === pageId ? loaded.detail : null;
  const [error, setError] = useState<string | null>(null);
  const [pane, setPane] = useState<Pane>("comments");
  const [at, setAt] = useState(0);
  const [summarising, setSummarising] = useState(false);
  const [draft, setDraft] = useState<Draft | null>(null);
  const player = useRef<PlayerHandle>(null);
  const drafts = useMemo(() => new DraftClient(handshake), [handshake]);

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

  const summarise = () => void run(() => drafts.generate(pageId));

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
  }, [library, drafts, pageId, say, t]);

  if (error) {
    return (
      <div className="p-5">
        <p className="border-danger/30 bg-danger-soft text-danger text-meta rounded-lg border px-3 py-2">
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
  const note = summary.kind === "note";

  const meta = (
    <>
      <Link to="/library" search={{}} className="text-fg-dim hover:text-fg text-meta">
        ← {t("meeting.back")}
      </Link>
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

  if (note) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="px-5 pt-5 pb-3">{meta}</div>
        <NoteEditor
          key={pageId}
          id={pageId}
          className="border-line border-t"
          onRemoved={() => void navigate({ to: "/library", search: {} })}
        />
      </div>
    );
  }

  return (
    <div className="p-5">
      {meta}

      <div className="mt-5 grid gap-4 lg:grid-cols-[1fr_380px]">
        <div className="space-y-4">
          <Player lanes={lanes} marks={marks} onTime={setAt} ref={player} />

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

          {/* Under the summary, because everything here is asked *about* what the meeting
              concluded. Writing the follow-up email is not a feature of its own — it is one of
              the things people ask for, and what comes back is a note like any other. */}
          <AskPanel meeting={pageId} />

          {sections.length === 0 ? (
            <Card>
              <CardBody className="pt-4 text-center">
                <p className="text-fg-faint">{t("meeting.no_summary_yet")}</p>
                <Button
                  className="mt-3"
                  variant="primary"
                  busy={summarising}
                  onClick={() => void summarise()}
                >
                  {t("meeting.summarise_now")}
                </Button>
              </CardBody>
            </Card>
          ) : (
            sections
              // Unapproved sections belong to the draft panel; drawing them here as well would
              // show the same paragraph twice.
              .filter((section) => !section.draft)
              .map((section) => (
                <Card key={section.heading}>
                  <CardHeader title={section.heading} />
                  <CardBody>
                    <p className="leading-relaxed whitespace-pre-wrap">{readable(section.body)}</p>
                  </CardBody>
                </Card>
              ))
          )}
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
