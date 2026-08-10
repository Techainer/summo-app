import { useCallback, useEffect, useMemo, useState } from "react";

import { Card, CardBody, CardHeader, SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";
import {
  ReportClient,
  duration,
  share,
  shiftDay,
  today,
  type Report,
} from "../lib/report";

type Range = "day" | "week" | "month";

const RANGES = [
  { value: "day" as const, label: "Hôm nay" },
  { value: "week" as const, label: "7 ngày" },
  { value: "month" as const, label: "30 ngày" },
];

const DAYS: Record<Range, number> = { day: 0, week: 6, month: 29 };

/**
 * Where the time went, and what is still open.
 *
 * Every number here is arithmetic over the vault, so the screen is honest about a quiet period
 * rather than inventing activity: an empty range says so.
 */
export function AnalyticsScreen() {
  const { handshake } = useEngine();
  const client = useMemo(() => new ReportClient(handshake), [handshake]);
  const [range, setRange] = useState<Range>("week");
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const to = today();
    const from = shiftDay(to, -DAYS[range]);
    try {
      setReport(await client.between(from, to));
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client, range]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="mx-auto max-w-3xl space-y-4 p-5">
      <div className="flex items-center gap-3">
        <h1 className="text-xl font-semibold tracking-tight">Thống kê</h1>
        <SegmentedControl
          className="ml-auto"
          label="Khoảng thời gian"
          options={RANGES}
          value={range}
          onChange={setRange}
          size="sm"
        />
      </div>

      {error && (
        <p className="rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
          {error}
        </p>
      )}

      {report && report.meetings.length === 0 && (
        <p className="mt-16 text-center text-fg-faint">Không có buổi họp nào trong khoảng này.</p>
      )}

      {report && report.meetings.length > 0 && (
        <>
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
            <Metric label="Buổi họp" value={String(report.meetings.length)} />
            <Metric label="Thời gian" value={duration(report.total_seconds)} />
            <Metric label="Việc còn treo" value={String(report.open_actions.length)} />
            <Metric label="Chưa tóm tắt" value={String(report.without_summary.length)} />
          </div>

          {report.people.length > 0 && (
            <Card>
              <CardHeader title="Thời gian với ai" count={`${report.people.length} người`} />
              <CardBody className="space-y-2">
                {report.people.slice(0, 8).map((person) => (
                  <div key={person.name} className="flex items-center gap-3">
                    <span className="w-32 shrink-0 truncate text-sm">{person.name}</span>
                    <span className="h-2 flex-1 overflow-hidden rounded-full bg-bg-soft">
                      <span
                        className="block h-full rounded-full bg-accent"
                        style={{ width: `${share(person.seconds, report.total_seconds)}%` }}
                      />
                    </span>
                    <span className="tabular w-24 shrink-0 text-right text-[13px] text-fg-dim">
                      {duration(person.seconds)}
                    </span>
                  </div>
                ))}
              </CardBody>
            </Card>
          )}

          {report.open_actions.length > 0 && (
            <Card>
              <CardHeader
                title="Còn phải làm"
                count={`${report.open_actions.length} việc`}
              />
              <CardBody className="space-y-1.5">
                {report.open_actions.map((action, i) => (
                  <div
                    key={`${action.meeting}-${i}`}
                    className="flex items-baseline gap-2 text-sm"
                  >
                    <span aria-hidden="true" className="text-fg-faint">
                      ☐
                    </span>
                    <span className="flex-1">{action.text}</span>
                    <span className="shrink-0 text-[12px] text-fg-faint">
                      {action.meeting_title}
                    </span>
                  </div>
                ))}
              </CardBody>
            </Card>
          )}

          {report.quiet_days.length > 0 && (
            <p className="text-[13px] text-fg-faint">
              Không họp: {report.quiet_days.join(", ")}
            </p>
          )}
        </>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <Card className="p-3">
      <p className="text-[12px] text-fg-faint">{label}</p>
      <p className="tabular mt-1 text-xl font-semibold">{value}</p>
    </Card>
  );
}
