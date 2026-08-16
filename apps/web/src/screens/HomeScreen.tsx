import { useNavigate } from "@tanstack/react-router";
import { m } from "motion/react";
import { ArrowRight, CircleAlert, FileUp, PencilLine, Sparkles } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Recent } from "../components/library/Recent";
import {
  Avatar,
  Button,
  Card,
  CardBody,
  CardHeader,
  Page,
  PageGlow,
  Ticker,
  Wave,
} from "../components/ui";
import { useI18n, useT } from "../i18n/context";
import { clock } from "../lib/clock";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/duration";
import { useEngine } from "../lib/engine-context";
import { useErrorText } from "../lib/errors";
import type { MeetingSummary } from "../lib/library";
import { listItem, stagger } from "../lib/motion";
import { ReportClient, shiftDay, today, type ActionItem, type Report } from "../lib/report";

/**
 * What is worth seeing first.
 *
 * The landing screen was the recorder: one button, and several hundred pixels of nothing. That is
 * an honest picture of a *recorder* and a poor one of a workspace — it says the only thing you can
 * do here is start, when most openings of this app are to pick something up rather than begin.
 *
 * So this aggregates, in the order a person actually wants it:
 *
 * 1. **Capture**, because it is still the one irreversible action and must never be hunted for.
 * 2. **Cần bạn** — the queue. Drafts nobody confirmed, recordings with no summary, overdue tasks.
 *    Every one of these is something the machine did and is now waiting on a human, which is the
 *    only kind of backlog worth putting on a home screen.
 * 3. **Gần đây**, of every kind, because "the thing I had open yesterday" is the most common reason
 *    to open a notes app at all.
 *
 * The second pass at this screen was about weight rather than content. A card holding one small
 * circle in the middle of four hundred pixels is still a void, only now it has a border around it;
 * so capture became a wide panel that shows what it is doing — a live waveform and a running clock
 * while recording, its own light when idle — and the rows underneath grew the things that make a
 * list scannable: a silhouette per recording, a face per name, a number per fact.
 */
export function HomeScreen() {
  const t = useT();
  const { locale } = useI18n();
  const say = useErrorText();
  const navigate = useNavigate();
  const { handshake, session, elapsed, toggle, transcript } = useEngine();

  const reports = useMemo(() => new ReportClient(handshake), [handshake]);

  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        // A fortnight rather than a week: the queue is "what is still waiting", and something
        // waiting for nine days is exactly what a seven-day window would hide.
        const summary = await reports.between(shiftDay(today(), -14), today());
        if (cancelled) return;
        setReport(summary);
      } catch (e) {
        if (!cancelled) setError(say(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [reports, say]);

  const open = useCallback(
    (entry: MeetingSummary) => {
      void navigate({ to: "/pages/$pageId", params: { pageId: entry.id } });
    },
    [navigate],
  );

  const waiting = queue(report);

  // The last three lines, newest at the bottom. Three because the card has room for three without
  // pushing the controls off it, and because a person glancing at it wants "is it hearing me",
  // not a transcript.
  const recent = transcript.segments.slice(-3);

  return (
    <Page
      data-testid="home"
      eyebrow={t(greetingKey())}
      title={t("home.heading")}
      subtitle={t("home.subheading")}
      aside={
        // Three numbers, because a workspace should say how much is in it. All three are already
        // computed for the analytics screen; nothing new is fetched to show them.
        report ? (
          <dl className="flex gap-6">
            <Stat value={String(report.meetings.length)} label={t("home.stat_meetings")} />
            <Stat
              value={formatDuration(report.total_seconds, locale, "short")}
              label={t("home.stat_time")}
            />
            <Stat value={String(report.open_actions.length)} label={t("home.stat_open")} />
          </dl>
        ) : undefined
      }
    >
      <PageGlow />

      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-[var(--radius-card)] border px-3 py-2">
          {error}
        </p>
      )}

      <m.div
        initial="hidden"
        animate="shown"
        transition={stagger(4)}
        className="grid gap-4 lg:grid-cols-[1.35fr_1fr]"
      >
        {/* Capture. Still the one irreversible action, so it keeps the largest target on the
              screen — but it now shows the state it is in rather than only offering to change it. */}
        <m.div variants={listItem}>
          <Card
            className={cn(
              "relative h-full overflow-hidden transition-shadow duration-300",
              session.recording && "shadow-[var(--glow-rec)]",
            )}
          >
            <div
              aria-hidden="true"
              className={cn(
                "pointer-events-none absolute inset-0 bg-[image:var(--gradient-capture)] transition-opacity duration-500",
                session.recording ? "opacity-100" : "opacity-40",
              )}
            />
            <CardBody className="relative flex h-full flex-col gap-5 p-6">
              <div className="flex items-center gap-4">
                <button
                  type="button"
                  onClick={toggle}
                  aria-pressed={session.recording}
                  aria-label={session.recording ? t("record.stop") : t("record.start")}
                  className={cn(
                    "grid size-16 shrink-0 place-items-center rounded-full border-2 transition-all duration-200",
                    "focus-visible:border-accent focus:outline-none",
                    session.recording
                      ? "border-rec bg-rec-soft"
                      : "border-line-strong bg-bg-elevated hover:border-rec hover:scale-105",
                  )}
                >
                  <span
                    className={cn(
                      "bg-rec block transition-all duration-200",
                      session.recording ? "size-5 rounded-md" : "size-8 rounded-full",
                    )}
                  />
                </button>

                <div className="min-w-0">
                  <p className="text-title font-semibold">
                    {session.recording ? t("home.recording_now") : t("record.start")}
                  </p>
                  <p className="text-fg-dim text-meta mt-0.5">
                    {session.recording ? (
                      <span className="tabular">{clock(elapsed)}</span>
                    ) : (
                      t("home.capture_hint")
                    )}
                  </p>
                </div>
              </div>

              {/* Words, while they are being said.
                  
                  Pressing record here used to start a recording and show a waveform — and the
                  waveform is honest about *sound* and says nothing about *words*. Somebody who has
                  just installed the app presses record, says a sentence, and has no way to tell
                  whether recognition is working at all until they stop. The last few lines answer
                  that in the place the question is asked; the full transcript is one click away on
                  the record screen, which is where a whole meeting belongs. */}
              {session.recording && recent.length > 0 && (
                <ul className="min-h-0 flex-1 space-y-1 overflow-hidden" aria-live="polite">
                  {recent.map((segment) => (
                    <li
                      key={`${segment.lane}-${segment.seq}`}
                      // The same marker the transcript screen uses. A line is a line wherever it is
                      // drawn, and a test that asks "did speech reach the screen" was failing here
                      // against a working recogniser purely because this list had no name.
                      data-testid="transcript-line"
                      data-source={segment.source}
                      className={cn(
                        "text-body truncate",
                        segment.source === "partial" ? "text-fg-dim" : "text-fg",
                      )}
                    >
                      {segment.text}
                    </li>
                  ))}
                </ul>
              )}

              <div
                className={cn(
                  "transition-colors duration-300",
                  session.recording && recent.length > 0 ? "h-10" : "min-h-16 flex-1",
                  session.recording ? "text-rec" : "text-accent/30",
                )}
              >
                {/* Its own silhouette rather than a flat line: equal bars read as a dotted rule
                    somebody left in, not as sound. Idle it breathes, because forty motionless grey
                    bars across a third of the home screen read as a meter that has stopped
                    working. */}
                <Wave
                  seed="capture"
                  bars={40}
                  live={session.recording}
                  breathe={!session.recording}
                />
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() =>
                    void navigate({ to: "/record", search: { source: "upload" } as const })
                  }
                >
                  <FileUp aria-hidden="true" className="me-1 size-3.5" />
                  {t("record.tab_upload")}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void navigate({ to: "/notes", search: { open: undefined } })}
                >
                  <PencilLine aria-hidden="true" className="me-1 size-3.5" />
                  {t("home.write")}
                </Button>
                <p className="text-fg-faint text-micro ms-auto hidden sm:block">
                  {t("home.shortcut_hint")}
                </p>
              </div>
            </CardBody>
          </Card>
        </m.div>

        {/* The queue. Everything the machine has done and is now waiting on a person for. */}
        <m.div variants={listItem}>
          <Card className="h-full">
            <CardHeader
              title={
                <span className="flex items-center gap-2">
                  <CircleAlert aria-hidden="true" className="text-ai size-4" />
                  {t("home.waiting")}
                </span>
              }
              count={waiting.length > 0 ? waiting.length : undefined}
            />
            <CardBody>
              {waiting.length === 0 ? (
                <p className="text-fg-faint text-meta py-6 text-center">{t("home.all_clear")}</p>
              ) : (
                <ul className="space-y-1">
                  {waiting.slice(0, 5).map((item) => (
                    <li key={`${item.kind}-${item.key}`}>
                      <button
                        type="button"
                        onClick={() => void navigate({ to: item.to })}
                        className="hover:bg-bg-elevated group flex w-full items-center gap-2.5 rounded-[var(--radius-card)] px-2.5 py-2 text-left transition-colors"
                      >
                        <span
                          className={cn(
                            "size-1.5 shrink-0 rounded-full",
                            item.kind === "overdue" ? "bg-rec" : "bg-blocked",
                          )}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="text-body block truncate">{item.text}</span>
                          <span className="text-fg-faint text-micro">{t(item.labelKey)}</span>
                        </span>
                        {item.owner && <Avatar name={item.owner} size="sm" />}
                        <ArrowRight
                          aria-hidden="true"
                          className="text-fg-faint size-3.5 shrink-0 opacity-0 transition-opacity group-hover:opacity-100"
                        />
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </CardBody>
          </Card>
        </m.div>

        {/* Recent, of every kind. The most common reason to open a notes app is to pick something
              back up, and until now that took two clicks through a screen called Thư viện. */}
        <m.div variants={listItem} className="lg:col-span-2">
          <Card>
            <CardHeader
              title={t("home.recent")}
              actions={
                <Button size="sm" variant="ghost" onClick={() => void navigate({ to: "/library" })}>
                  {t("home.see_all")}
                </Button>
              }
            />
            <CardBody>
              <Recent onOpen={open} />
            </CardBody>
          </Card>
        </m.div>

        {/* What the assistant is for, said once, where somebody opening the app will read it. */}
        <m.div variants={listItem} className="lg:col-span-2">
          <Card className="border-ai/25 relative overflow-hidden">
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-0 bg-[image:var(--gradient-ai)]"
            />
            <CardBody className="relative flex flex-wrap items-center gap-3 py-4">
              <span className="bg-ai-soft ring-ai/25 grid size-9 shrink-0 place-items-center rounded-full ring-1">
                <Sparkles aria-hidden="true" className="text-ai size-4" />
              </span>
              <p className="text-meta text-fg-dim min-w-0 flex-1">{t("home.assistant_hint")}</p>
              <Button size="sm" variant="secondary" onClick={() => void navigate({ to: "/chat" })}>
                {t("home.ask")}
                <ArrowRight aria-hidden="true" className="ms-1 size-3.5" />
              </Button>
            </CardBody>
          </Card>
        </m.div>
      </m.div>
    </Page>
  );
}

/** One number and what it counts. `dl` because that is what this is. */
function Stat({ value, label }: { value: string; label: string }) {
  return (
    <div className="text-end">
      <dt className="text-fg-faint text-micro">{label}</dt>
      <dd className="nums text-title mt-0.5 font-semibold">
        <Ticker value={value} />
      </dd>
    </div>
  );
}

/**
 * Morning, afternoon or evening, by the clock on this machine.
 *
 * Local time, like everything else that says "today" here: a greeting computed in UTC would wish
 * somebody in Hanoi good evening over breakfast.
 */
function greetingKey(): string {
  const hour = new Date().getHours();
  if (hour < 12) return "home.greeting_morning";
  if (hour < 18) return "home.greeting_afternoon";
  return "home.greeting_evening";
}

/** One thing waiting on a person. */
interface Waiting {
  kind: "overdue" | "unsummarised";
  key: string;
  text: string;
  labelKey: string;
  to: "/tasks" | "/library";
  /** Who owes it, when anyone does. Only tasks have one. */
  owner?: string;
}

/**
 * The backlog, most urgent first.
 *
 * Overdue tasks before unsummarised recordings, because one is a commitment somebody made to
 * another person and the other is a chore. Capped by the caller rather than here, so the count in
 * the header is the true one even when the list is shortened.
 */
function queue(report: Report | null): Waiting[] {
  if (!report) return [];
  const now = today();

  const overdue: Waiting[] = report.open_actions
    .filter((action: ActionItem) => action.day < now)
    .map((action) => ({
      kind: "overdue" as const,
      key: `${action.meeting}-${action.text}`,
      text: action.text,
      labelKey: "home.overdue",
      to: "/tasks" as const,
      owner: action.owner ?? undefined,
    }));

  const unsummarised: Waiting[] = report.without_summary.map((title) => ({
    kind: "unsummarised" as const,
    key: title,
    text: title,
    labelKey: "home.no_summary",
    to: "/library" as const,
  }));

  return [...overdue, ...unsummarised];
}
