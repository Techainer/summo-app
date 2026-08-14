import { ChartNoAxesColumn, Square } from "lucide-react";
import { motion } from "motion/react";
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
import { GENTLE } from "../lib/motion";
import { useLoad } from "../lib/use-load";
import { useI18n } from "../i18n/context";
import { formatDuration } from "../lib/duration";
import { useEngine } from "../lib/engine-context";
import { ReportClient, share, shiftDay, today } from "../lib/report";

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

  return (
    <Page
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

      {report && report.meetings.length === 0 && (
        <Empty icon={ChartNoAxesColumn} title={t("analytics.empty")} />
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
                      <motion.span
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

          {report.quiet_days.length > 0 && (
            <p className="text-fg-faint text-meta">
              {t("analytics.quiet_days", {
                days: report.quiet_days.join(", "),
              })}
            </p>
          )}
        </>
      )}
    </Page>
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
