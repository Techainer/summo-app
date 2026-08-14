import { Link, useParams } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Comments } from "../components/meeting/Comments";
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
 * One meeting: what was said, what it meant, and what to do about it.
 *
 * Laid out like the Coreto reference — content in the centre, a rail on the right that toggles
 * between the written notes and the raw transcript. The player and comment threads arrive with the
 * rest of Phase 1; this is the shell they hang on.
 */
export function MeetingScreen() {
  const { t, locale } = useI18n();
  const say = useErrorText();
  const { meetingId } = useParams({ from: "/meetings/$meetingId" });
  const { library, handshake } = useEngine();
  // Stored with the meeting it belongs to, so switching meetings does not need an effect to blank
  // it first. The `setDetail(null)` that used to do that ran synchronously inside the effect — one
  // extra render per navigation, and a frame in which the new meeting's title sat above the old
  // meeting's transcript.
  const [loaded, setLoaded] = useState<{ id: string; detail: MeetingDetail } | null>(null);
  const detail = loaded?.id === meetingId ? loaded.detail : null;
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
      setLoaded({ id: meetingId, detail: await library.detail(meetingId) });
      setError(null);
    },
    [library, meetingId],
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
          url: url(handshake, `/meetings/${encodeURIComponent(meetingId)}/audio/${key}`),
        };
      }),
    [detail?.audio, handshake, meetingId, t],
  );

  const marks = useMemo(
    () => (detail?.transcript ?? []).map((segment) => segment.t0),
    [detail?.transcript],
  );

  const summarise = () => void run(() => drafts.generate(meetingId));

  useEffect(() => {
    let cancelled = false;
    library
      .detail(meetingId)
      .then((d) => !cancelled && setLoaded({ id: meetingId, detail: d }))
      .catch((e: unknown) => {
        if (!cancelled) setError(say(e));
      });
    // A meeting may already have a summary nobody has agreed to.
    drafts
      .get(meetingId)
      .then((d) => !cancelled && setDraft(d))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [library, drafts, meetingId, say, t]);

  if (error) {
    return (
      <div className="p-5">
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
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

  return (
    <div className="p-5">
      <Link to="/library" search={{}} className="text-fg-dim hover:text-fg text-meta">
        ← {t("meeting.back")}
      </Link>

      <div className="mt-2">
        <h1 className="text-2xl font-semibold tracking-tight">{summary.title}</h1>
        <div className="text-fg-dim text-meta mt-2 flex flex-wrap items-center gap-2">
          <Pill>{summary.day}</Pill>
          <Pill>{formatDuration(summary.duration, locale)}</Pill>
          {summary.folder && <Pill>{summary.folder}</Pill>}
          {summary.tags.map((tag) => (
            <Pill key={tag}>#{tag}</Pill>
          ))}
        </div>
      </div>

      <div className="mt-5 grid gap-4 lg:grid-cols-[1fr_380px]">
        <div className="space-y-4">
          <Player lanes={lanes} marks={marks} onTime={setAt} ref={player} />

          {draft && (
            <DraftPanel
              draft={draft}
              busy={summarising}
              onRefine={(heading, selection, instruction) =>
                void run(() => drafts.refine(meetingId, heading, selection, instruction))
              }
              onChat={(message) => void run(() => drafts.chat(meetingId, message))}
              onConfirm={() =>
                void run(async () => {
                  await drafts.confirm(meetingId);
                  return null;
                })
              }
              onDiscard={() =>
                void run(async () => {
                  await drafts.discard(meetingId);
                  return null;
                })
              }
            />
          )}

          {/* Under the summary, because everything here is asked *about* what the meeting
              concluded. Writing the follow-up email is not a feature of its own — it is one of
              the things people ask for, and what comes back is a note like any other. */}
          <AskPanel meeting={meetingId} />

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
                  meeting={meetingId}
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
