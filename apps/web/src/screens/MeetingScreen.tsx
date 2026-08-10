import { Link, useParams } from "@tanstack/react-router";
import { useEffect, useMemo, useRef, useState } from "react";

import { Player, type PlayerHandle } from "../components/meeting/Player";
import { TranscriptChips } from "../components/meeting/TranscriptChips";
import { Button, Card, CardBody, CardHeader, SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";
import { url, type MeetingDetail } from "../lib/library";
import { duration } from "../lib/report";

type Pane = "notes" | "transcript";

const PANES = [
  { value: "notes" as const, label: "Ghi chú" },
  { value: "transcript" as const, label: "Transcript" },
];

/**
 * One meeting: what was said, what it meant, and what to do about it.
 *
 * Laid out like the Coreto reference — content in the centre, a rail on the right that toggles
 * between the written notes and the raw transcript. The player and comment threads arrive with the
 * rest of Phase 1; this is the shell they hang on.
 */
export function MeetingScreen() {
  const { meetingId } = useParams({ from: "/meetings/$meetingId" });
  const { library, handshake } = useEngine();
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pane, setPane] = useState<Pane>("notes");
  const [at, setAt] = useState(0);
  const [summarising, setSummarising] = useState(false);
  const player = useRef<PlayerHandle>(null);

  // The daemon reports file names (`mic.opus`); the route takes lane names. Strip the extension
  // here rather than teaching the player about container formats.
  const lanes = useMemo(
    () =>
      (detail?.audio ?? []).map((file) => {
        const key = file.replace(/\.[^.]+$/, "");
        return {
          key,
          label: key === "mic" ? "Mic" : "Hệ thống",
          url: url(handshake, `/meetings/${encodeURIComponent(meetingId)}/audio/${key}`),
        };
      }),
    [detail?.audio, handshake, meetingId],
  );

  const marks = useMemo(
    () => (detail?.transcript ?? []).map((segment) => segment.t0),
    [detail?.transcript],
  );

  const summarise = async () => {
    setSummarising(true);
    try {
      await fetch(url(handshake, `/meetings/${encodeURIComponent(meetingId)}/summarize`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({}),
      }).then(async (r) => {
        if (!r.ok) {
          const body = (await r.json().catch(() => null)) as { error?: string } | null;
          throw new Error(body?.error ?? `${r.status}`);
        }
      });
      setDetail(await library.detail(meetingId));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setSummarising(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    setDetail(null);
    library
      .detail(meetingId)
      .then((d) => !cancelled && setDetail(d))
      .catch((e: unknown) => {
        if (!cancelled) setError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [library, meetingId]);

  if (error) {
    return (
      <div className="p-5">
        <p className="rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
          {error}
        </p>
      </div>
    );
  }

  if (!detail) {
    return <p className="mt-24 text-center text-fg-faint">Đang mở…</p>;
  }

  const { summary, sections, transcript } = detail;

  return (
    <div className="p-5">
      <Link
        to="/library"
        search={{}}
        className="text-[13px] text-fg-dim hover:text-fg"
      >
        ← Thư viện
      </Link>

      <div className="mt-2">
        <h1 className="text-2xl font-semibold tracking-tight">{summary.title}</h1>
        <div className="mt-2 flex flex-wrap items-center gap-2 text-[13px] text-fg-dim">
          <Pill>{summary.day}</Pill>
          <Pill>{duration(summary.duration)}</Pill>
          {summary.folder && <Pill>{summary.folder}</Pill>}
          {summary.tags.map((tag) => (
            <Pill key={tag}>#{tag}</Pill>
          ))}
        </div>
      </div>

      <div className="mt-5 grid gap-4 lg:grid-cols-[1fr_380px]">
        <div className="space-y-4">
          <Player lanes={lanes} marks={marks} onTime={setAt} ref={player} />

          {sections.length === 0 ? (
            <Card>
              <CardBody className="pt-4 text-center">
                <p className="text-fg-faint">Chưa có tóm tắt cho buổi họp này.</p>
                <Button
                  className="mt-3"
                  variant="primary"
                  busy={summarising}
                  onClick={() => void summarise()}
                >
                  Tóm tắt ngay
                </Button>
              </CardBody>
            </Card>
          ) : (
            sections.map((section) => (
              <Card key={section.heading}>
                <CardHeader title={section.heading} />
                <CardBody>
                  <p className="whitespace-pre-wrap leading-relaxed">{section.body}</p>
                </CardBody>
              </Card>
            ))
          )}
        </div>

        <aside className="space-y-3">
          <SegmentedControl
            label="Nội dung bên phải"
            options={PANES}
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
              <CardBody className="pt-4">
                <p className="text-[13px] text-fg-faint">
                  Ghi chú riêng cho buổi họp sẽ xuất hiện ở đây.
                </p>
              </CardBody>
            )}
          </Card>
        </aside>
      </div>
    </div>
  );
}

function Pill({ children }: { children: React.ReactNode }) {
  return (
    <span className="rounded-full border border-line bg-bg-soft px-2.5 py-1">{children}</span>
  );
}
