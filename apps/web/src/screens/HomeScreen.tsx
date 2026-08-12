import { useNavigate } from "@tanstack/react-router";
import { motion } from "motion/react";
import { CircleAlert, FileUp, Mic, NotebookPen, PencilLine, Sparkles, Waves } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button, Card, CardBody, CardHeader } from "../components/ui";
import { useI18n, useT } from "../i18n/context";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/duration";
import { useEngine } from "../lib/engine-context";
import { useErrorText } from "../lib/errors";
import { LibraryClient, dayLabel, localDay, timeOfDay, type MeetingSummary } from "../lib/library";
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
 * Everything here is derived from `/library` and `/report`, both of which already existed and
 * neither of which anything was showing together.
 */
export function HomeScreen() {
  const t = useT();
  const { locale } = useI18n();
  const say = useErrorText();
  const navigate = useNavigate();
  const { handshake, session, toggle } = useEngine();

  const library = useMemo(() => new LibraryClient(handshake), [handshake]);
  const reports = useMemo(() => new ReportClient(handshake), [handshake]);

  const [recent, setRecent] = useState<MeetingSummary[] | null>(null);
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        // A fortnight rather than a week: the queue is "what is still waiting", and something
        // waiting for nine days is exactly what a seven-day window would hide.
        const [view, summary] = await Promise.all([
          library.view({ group: "day" }),
          reports.between(shiftDay(today(), -14), today()),
        ]);
        if (cancelled) return;
        setRecent(view.groups.flatMap((group) => group.meetings).slice(0, 6));
        setReport(summary);
      } catch (e) {
        if (!cancelled) setError(say(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [library, reports, say]);

  const open = useCallback(
    (entry: MeetingSummary) => {
      void navigate(
        entry.kind === "note"
          ? { to: "/notes" }
          : { to: "/meetings/$meetingId", params: { meetingId: entry.id } },
      );
    },
    [navigate],
  );

  const waiting = queue(report);

  return (
    <div className="mx-auto max-w-5xl p-5" data-testid="home">
      <h1 className="text-display font-semibold">{t("home.heading")}</h1>
      <p className="text-fg-faint text-meta mt-1">{t("home.subheading")}</p>

      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta mt-4 rounded-[var(--radius-card)] border px-3 py-2">
          {error}
        </p>
      )}

      <motion.div
        initial="hidden"
        animate="shown"
        transition={stagger(4)}
        className="mt-5 grid gap-4 lg:grid-cols-[1.1fr_1fr]"
      >
        {/* Capture. Still the one irreversible action, so it keeps the largest target on the
            screen even though it is no longer the whole screen. */}
        <motion.div variants={listItem}>
          <Card className="h-full">
            <CardBody className="flex h-full flex-col items-center justify-center gap-4 pt-6 pb-7">
              <button
                type="button"
                onClick={toggle}
                aria-pressed={session.recording}
                aria-label={session.recording ? t("record.stop") : t("record.start")}
                className={cn(
                  "grid size-20 place-items-center rounded-full border-2 transition-all duration-200",
                  "focus-visible:border-accent focus:outline-none",
                  session.recording
                    ? "border-rec bg-rec-soft scale-105"
                    : "border-line-strong bg-bg-elevated hover:border-rec hover:scale-105",
                )}
              >
                <span
                  className={cn(
                    "bg-rec block transition-all duration-200",
                    session.recording ? "size-6 rounded-md" : "size-10 rounded-full",
                  )}
                />
              </button>
              <p className="text-fg-faint text-meta text-center">{t("home.capture_hint")}</p>
              <div className="flex flex-wrap justify-center gap-2">
                <Button size="sm" variant="secondary" onClick={() => void navigate({ to: "/" })}>
                  <FileUp aria-hidden="true" className="me-1 size-3.5" />
                  {t("record.tab_upload")}
                </Button>
                <Button
                  size="sm"
                  variant="secondary"
                  onClick={() => void navigate({ to: "/notes" })}
                >
                  <PencilLine aria-hidden="true" className="me-1 size-3.5" />
                  {t("home.write")}
                </Button>
              </div>
            </CardBody>
          </Card>
        </motion.div>

        {/* The queue. Everything the machine has done and is now waiting on a person for. */}
        <motion.div variants={listItem}>
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
                <ul className="space-y-1.5">
                  {waiting.slice(0, 5).map((item) => (
                    <li key={`${item.kind}-${item.key}`}>
                      <button
                        type="button"
                        onClick={() => void navigate({ to: item.to })}
                        className="hover:bg-bg-elevated flex w-full items-start gap-2.5 rounded-[var(--radius-card)] px-2 py-1.5 text-left transition-colors"
                      >
                        <span
                          className={cn(
                            "mt-1.5 size-1.5 shrink-0 rounded-full",
                            item.kind === "overdue" ? "bg-rec" : "bg-blocked",
                          )}
                        />
                        <span className="min-w-0 flex-1">
                          <span className="text-body block truncate">{item.text}</span>
                          <span className="text-fg-faint text-micro">{t(item.labelKey)}</span>
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </CardBody>
          </Card>
        </motion.div>

        {/* Recent, of every kind. The most common reason to open a notes app is to pick something
            back up, and until now that took two clicks through a screen called Thư viện. */}
        <motion.div variants={listItem} className="lg:col-span-2">
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
              {recent === null ? (
                <p className="text-fg-faint text-meta py-6 text-center">{t("common.loading")}</p>
              ) : recent.length === 0 ? (
                <p className="text-fg-faint text-meta py-6 text-center">{t("library.empty")}</p>
              ) : (
                <ul className="grid gap-1.5 sm:grid-cols-2">
                  {recent.map((entry) => (
                    <li key={entry.id}>
                      <button
                        type="button"
                        onClick={() => open(entry)}
                        className="hover:bg-bg-elevated flex w-full items-center gap-3 rounded-[var(--radius-card)] px-2.5 py-2 text-left transition-colors"
                      >
                        <span className="bg-bg-soft ring-line grid size-9 shrink-0 place-items-center rounded-full ring-1">
                          {entry.kind === "note" ? (
                            <NotebookPen aria-hidden="true" className="text-fg-faint size-4" />
                          ) : (
                            <Waves aria-hidden="true" className="text-fg-faint size-4" />
                          )}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="text-body block truncate font-medium">
                            {entry.title}
                          </span>
                          <span className="text-fg-faint text-micro">
                            {dayLabel(entry.day, localDay(), {
                              locale,
                              today: t("library.today"),
                              yesterday: t("library.yesterday"),
                              week: t("library.week"),
                              unfiled: t("library.unfiled_group"),
                            })}
                            {entry.kind === "meeting" && entry.duration > 0 && (
                              <> · {formatDuration(entry.duration, locale, "short")}</>
                            )}
                            {entry.kind === "meeting" && <> · {timeOfDay(entry.date)}</>}
                          </span>
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </CardBody>
          </Card>
        </motion.div>

        {/* What the assistant is for, said once, where somebody opening the app will read it. */}
        <motion.div variants={listItem} className="lg:col-span-2">
          <Card className="border-ai/25 bg-ai-soft">
            <CardBody className="flex flex-wrap items-center gap-3 pt-4">
              <Sparkles aria-hidden="true" className="text-ai size-4 shrink-0" />
              <p className="text-meta text-fg-dim min-w-0 flex-1">{t("home.assistant_hint")}</p>
              <Button size="sm" variant="secondary" onClick={() => void navigate({ to: "/chat" })}>
                <Mic aria-hidden="true" className="me-1 size-3.5" />
                {t("home.ask")}
              </Button>
            </CardBody>
          </Card>
        </motion.div>
      </motion.div>
    </div>
  );
}

/** One thing waiting on a person. */
interface Waiting {
  kind: "overdue" | "unsummarised";
  key: string;
  text: string;
  labelKey: string;
  to: "/tasks" | "/library";
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
