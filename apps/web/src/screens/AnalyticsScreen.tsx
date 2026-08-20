import { Link } from "@tanstack/react-router";
import { ChartNoAxesColumn, Square } from "lucide-react";
import { m } from "motion/react";
import { useMemo, useState } from "react";

import {
  Avatar,
  Card,
  CardBody,
  CardHeader,
  Empty,
  Page,
  PageGlow,
  SegmentedControl,
  Ticker,
} from "../components/ui";
import { cn } from "../lib/cn";
import { GENTLE } from "../lib/motion";
import { useLoad } from "../lib/use-load";
import { useI18n } from "../i18n/context";
import { formatDuration } from "../lib/duration";
import { useEngine } from "../lib/engine-context";
import { ReportClient, byDay, share, shiftDay, today } from "../lib/report";

type Range = "day" | "week" | "month";

// Labels are keys, resolved at render: a module-level array is built once, before any provider
// exists, so baking the text in would freeze the first language forever.
const RANGES = [
  { value: "day" as const, labelKey: "analytics.today" },
  { value: "week" as const, labelKey: "analytics.days_7" },
  { value: "month" as const, labelKey: "analytics.days_30" },
];

const DAYS: Record<Range, number> = { day: 0, week: 6, month: 29 };

/**
 * Where the time went, and what is still open.
 *
 * Every number here is arithmetic over the vault, so the screen is honest about a quiet period
 * rather than inventing activity: an empty range says so.
 */
export function AnalyticsScreen() {
  const { t, locale } = useI18n();
  const { handshake } = useEngine();
  const client = useMemo(() => new ReportClient(handshake), [handshake]);
  const [range, setRange] = useState<Range>("week");

  const { data: report, error } = useLoad(
    () => client.between(shiftDay(today(), -DAYS[range]), today()),
    [client, range],
  );
  const days = useMemo(() => (report ? byDay(report) : []), [report]);

  return (
    <Page
      // The frame fills the pane when there is nothing in it, so the one sentence this screen has
      // to say sits in the middle of it rather than at the top of four hundred pixels of nothing.
      fill={report?.meetings.length === 0}
      title={t("analytics.title")}
      actions={
        <SegmentedControl
          label={t("analytics.range")}
          options={RANGES.map((r) => ({
            value: r.value,
            label: t(r.labelKey),
          }))}
          value={range}
          onChange={setRange}
          size="sm"
        />
      }
    >
      <PageGlow />

      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {/* `full`, because when this is on screen it *is* the screen. Stacked at the top it left
          four hundred pixels of background under one sentence — which is what a quiet week looks
          like to anyone who takes a fortnight off, and what this screen looked like the morning the
          seeded fixture aged past its own window. */}
      {report && report.meetings.length === 0 && (
        <Empty
          full
          icon={ChartNoAxesColumn}
          title={t("analytics.empty")}
          hint={t("analytics.empty_hint")}
        />
      )}

      {report && report.meetings.length > 0 && (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Metric label={t("analytics.meetings")} value={String(report.meetings.length)} />
            <Metric
              label={t("analytics.time")}
              value={formatDuration(report.total_seconds, locale, "short")}
            />
            <Metric label={t("analytics.open_tasks")} value={String(report.open_actions.length)} />
            <Metric
              label={t("analytics.unsummarised")}
              value={String(report.without_summary.length)}
            />
          </div>

          {report.people.length > 0 && (
            <Card>
              <CardHeader
                title={t("analytics.time_with")}
                count={t("analytics.people_count", {
                  count: report.people.length,
                })}
              />
              <CardBody className="space-y-2">
                {report.people.slice(0, 8).map((person) => (
                  <div key={person.name} className="flex items-center gap-2.5">
                    <Avatar name={person.name} size="sm" />
                    <span className="w-28 shrink-0 truncate text-sm">{person.name}</span>
                    <span className="bg-bg-soft h-2 flex-1 overflow-hidden rounded-full">
                      {/* Grown from the left rather than drawn at length. A bar chart that
                          animates in reads as measured; one that appears reads as decoration. */}
                      <m.span
                        initial={{ scaleX: 0 }}
                        animate={{ scaleX: 1 }}
                        transition={GENTLE}
                        className="bg-accent block h-full origin-left rounded-full"
                        style={{
                          width: `${share(person.seconds, report.total_seconds)}%`,
                        }}
                      />
                    </span>
                    <span className="nums text-fg-dim text-meta w-28 shrink-0 text-right text-balance">
                      {formatDuration(person.seconds, locale, "short")}
                    </span>
                  </div>
                ))}
              </CardBody>
            </Card>
          )}

          {/* What this stretch of work was *about*.

              The daemon has counted tags into every report since reports existed and no screen had
              ever drawn them — the same shape as the retention policy the daemon enforced and the
              settings screen never offered. They are the one thing here that answers "what have I
              been spending my time on" rather than "how much", and each one is a link into the
              library filtered by it, so the answer is somewhere to go rather than a number. */}
          {report.tags.length > 0 && (
            <Card>
              <CardHeader
                title={t("analytics.tags")}
                count={t("analytics.tags_count", { count: report.tags.length })}
              />
              <CardBody className="flex flex-wrap gap-1.5">
                {report.tags.slice(0, 12).map(([tag, uses]) => (
                  <Link
                    key={tag}
                    to="/library"
                    search={{ tag }}
                    className="border-line bg-bg-soft hover:border-accent hover:text-accent text-meta inline-flex items-center gap-1.5 rounded-[var(--radius-pill)] border px-2.5 py-1 transition-colors"
                  >
                    #{tag}
                    <span className="text-fg-faint text-micro">
                      {t("analytics.tag_uses", { count: uses })}
                    </span>
                  </Link>
                ))}
              </CardBody>
            </Card>
          )}

          {report.open_actions.length > 0 && (
            <Card>
              <CardHeader
                title={t("analytics.todo")}
                count={t("analytics.tasks_count", {
                  count: report.open_actions.length,
                })}
              />
              <CardBody className="space-y-1.5">
                {report.open_actions.map((action, i) => (
                  <div key={`${action.meeting}-${i}`} className="flex items-baseline gap-2 text-sm">
                    <Square
                      aria-hidden="true"
                      className="text-fg-faint mt-0.5 size-3.5 shrink-0 stroke-[1.75]"
                    />
                    <span className="flex-1">{action.text}</span>
                    <span className="text-fg-faint text-micro shrink-0">
                      {action.meeting_title}
                    </span>
                  </div>
                ))}
              </CardBody>
            </Card>
          )}

          {/* When the work happened, which is the question this screen was opened with and the one
              it answered least well: four totals at the top, then several hundred pixels of nothing,
              then a comma-separated list of the dates with no meetings on them. The same facts as a
              strip make a quiet day a gap in a shape rather than a sentence to read. */}
          <Days days={days} locale={locale} label={t("analytics.by_day")} />

          {report.quiet_days.length > 0 && (
            <p className="text-fg-faint text-meta">
              {t("analytics.quiet_count", { count: report.quiet_days.length })}
            </p>
          )}
        </>
      )}
    </Page>
  );
}

/**
 * One column per day in the window, as tall as the time recorded on it.
 *
 * Scaled against the busiest day rather than against the total, because the comparison a person is
 * making is between days. A day with nothing on it keeps its column — a hairline at the baseline —
 * so the gaps are part of the shape instead of missing from it.
 */
function Days({
  days,
  locale,
  label,
}: {
  days: { day: string; seconds: number; count: number }[];
  locale: string;
  label: string;
}) {
  if (days.length < 2) return null;
  const peak = Math.max(...days.map((d) => d.seconds));
  const weekday = new Intl.DateTimeFormat(locale, { weekday: "narrow", timeZone: "UTC" });
  const full = new Intl.DateTimeFormat(locale, { dateStyle: "medium", timeZone: "UTC" });

  return (
    <Card>
      <CardHeader title={label} />
      <CardBody>
        <div className="flex h-24 items-end gap-1">
          {days.map((day) => {
            const at = new Date(`${day.day}T00:00:00Z`);
            return (
              // Capped, and centred in its share of the row. Seven columns stretched across a
              // wide pane read as blocks rather than bars; the cap keeps the shape the same at
              // every width the window takes.
              <div
                key={day.day}
                className="flex h-full flex-1 flex-col items-center justify-end gap-1"
              >
                <m.div
                  initial={{ scaleY: 0 }}
                  animate={{ scaleY: 1 }}
                  transition={GENTLE}
                  title={`${full.format(at)} — ${formatDuration(day.seconds, locale, "short")}`}
                  style={{
                    // A floor of two pixels: a day with one short meeting on it and a day with
                    // none must not look the same.
                    height: peak > 0 ? `${Math.max(2, (day.seconds / peak) * 100)}%` : "2px",
                  }}
                  className={cn(
                    "w-full max-w-8 origin-bottom rounded-sm",
                    day.seconds > 0 ? "bg-accent" : "bg-line",
                  )}
                />
                {/* Only when they fit. Thirty narrow columns with a letter under each is a row of
                    noise, and the bars carry the shape on their own. */}
                {days.length <= 14 && (
                  <span className="text-fg-faint text-micro text-center">{weekday.format(at)}</span>
                )}
              </div>
            );
          })}
        </div>
      </CardBody>
    </Card>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <Card className="p-3">
      <p className="text-fg-faint text-micro">{label}</p>
      {/* `text-balance` because these are small boxes holding a phrase, not a number.
          "1 giờ 27 phút" broke as "1 giờ 27" / "phút", which reads as two facts. Vietnamese has no
          abbreviation for `giờ` or `phút`, so `Intl`'s short form does nothing here and the fix has
          to be the wrap rather than the wording. */}
      <p className="nums mt-1 text-xl font-semibold text-balance">
        <Ticker value={value} />
      </p>
    </Card>
  );
}
