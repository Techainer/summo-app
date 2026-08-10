import { Link, useParams } from "@tanstack/react-router";
import { useEffect, useState } from "react";

import { Card, CardBody, CardHeader, SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";
import type { MeetingDetail } from "../lib/library";
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
  const { library } = useEngine();
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pane, setPane] = useState<Pane>("notes");

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

      <header className="mt-2">
        <h1 className="text-2xl font-semibold tracking-tight">{summary.title}</h1>
        <div className="mt-2 flex flex-wrap items-center gap-2 text-[13px] text-fg-dim">
          <Pill>{summary.day}</Pill>
          <Pill>{duration(summary.duration)}</Pill>
          {summary.folder && <Pill>{summary.folder}</Pill>}
          {summary.tags.map((tag) => (
            <Pill key={tag}>#{tag}</Pill>
          ))}
        </div>
      </header>

      <div className="mt-5 grid gap-4 lg:grid-cols-[1fr_360px]">
        <div className="space-y-4">
          {sections.length === 0 ? (
            <p className="text-fg-faint">Chưa có tóm tắt cho buổi họp này.</p>
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
          <Card>
            <CardBody className="max-h-[60vh] overflow-y-auto pt-4">
              {pane === "transcript" ? (
                transcript.length === 0 ? (
                  <p className="text-[13px] text-fg-faint">Không có transcript.</p>
                ) : (
                  <ol className="space-y-3">
                    {transcript.map((segment, i) => (
                      <li key={i}>
                        <p className="tabular text-[12px] text-fg-faint">
                          {segment.speaker ?? "—"}
                        </p>
                        <p className="mt-0.5 text-sm leading-relaxed">{segment.text}</p>
                      </li>
                    ))}
                  </ol>
                )
              ) : (
                <p className="text-[13px] text-fg-faint">
                  Ghi chú riêng cho buổi họp sẽ xuất hiện ở đây.
                </p>
              )}
            </CardBody>
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
