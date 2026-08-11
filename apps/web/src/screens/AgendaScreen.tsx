import { motion } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { Button } from "../components/ui";
import { useI18n } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { pickFile } from "../lib/imports";
import { AgendaClient, byDay, clock, length, service, type AgendaEntry } from "../lib/notes";

/**
 * What is on the calendar, and nothing more than that.
 *
 * The screen deliberately does not offer to record anything on a schedule. Summo reads calendars so
 * a meeting note can be titled after the meeting it was for; an app that starts listening because a
 * calendar said so is an app that records the therapy appointment somebody put in their work
 * calendar. The only actions here are *join* and *add a calendar*.
 *
 * Times render as the calendar wrote them — see `clock` in `lib/notes` for why that is UTC-read and
 * why it is right more often than the alternative.
 */
export function AgendaScreen() {
  const { handshake } = useEngine();
  const { t } = useI18n();
  const client = useMemo(() => new AgendaClient(handshake), [handshake]);

  const [entries, setEntries] = useState<AgendaEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [path, setPath] = useState("");
  const [name, setName] = useState("");

  const refresh = useCallback(async () => {
    try {
      setEntries(await client.list());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [client]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const add = async (from: string) => {
    const file = from.trim();
    if (!file) return;
    setError(null);
    try {
      // The name defaults to the file's own, so adding one calendar takes one action.
      const fallback = (file.split(/[/\\]/).pop() ?? "calendar").replace(/\.ics$/i, "");
      await client.addCalendar(file, name.trim() || fallback);
      setPath("");
      setName("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const browse = async () => {
    const chosen = await pickFile("iCalendar");
    if (chosen === null) {
      setError(t("import.no_dialog"));
      return;
    }
    await add(chosen);
  };

  const calendars = [...new Set(entries.map((e) => e.calendar))].sort();
  const grouped = byDay(entries).reverse();

  return (
    <div className="mx-auto w-full max-w-3xl px-6 py-8">
      <h1 className="text-lg font-semibold">{t("agenda.title")}</h1>
      <p className="mt-1 text-sm text-fg-dim">{t("agenda.hint")}</p>

      {error && (
        <p role="alert" className="mt-4 text-sm text-danger">
          {error}
        </p>
      )}

      <div className="mt-5 flex flex-wrap gap-2">
        <input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void add(path);
          }}
          placeholder={t("agenda.path_placeholder")}
          aria-label={t("agenda.path_label")}
          className="min-w-0 flex-1 rounded-xl border border-line bg-bg-soft px-3 py-2 text-sm outline-none focus:border-accent"
        />
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={t("agenda.name_placeholder")}
          aria-label={t("agenda.name_label")}
          className="w-36 rounded-xl border border-line bg-bg-soft px-3 py-2 text-sm outline-none focus:border-accent"
        />
        <Button variant="ghost" onClick={() => void browse()}>
          {t("import.browse")}
        </Button>
        <Button onClick={() => void add(path)} disabled={!path.trim()}>
          {t("agenda.add")}
        </Button>
      </div>

      {calendars.length > 0 && (
        <ul className="mt-3 flex flex-wrap gap-2">
          {calendars.map((calendar) => (
            <li
              key={calendar}
              className="flex items-center gap-1.5 rounded-full border border-line bg-bg-soft px-2.5 py-1 text-[13px]"
            >
              {calendar}
              <button
                type="button"
                aria-label={t("agenda.remove_calendar", { name: calendar })}
                onClick={() => void client.removeCalendar(calendar).then(refresh)}
                className="text-fg-faint hover:text-danger"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}

      {entries.length === 0 ? (
        <p className="mt-12 text-center text-fg-faint">{t("agenda.empty")}</p>
      ) : (
        <div className="mt-8 space-y-6">
          {grouped.map(([day, items]) => (
            <section key={day}>
              <h2 className="text-[11px] font-semibold uppercase tracking-wider text-fg-faint">
                {day}
              </h2>
              <ul className="mt-2 space-y-1.5">
                {items.map((entry) => (
                  <motion.li
                    key={`${entry.calendar}:${entry.uid}:${entry.start_epoch}`}
                    initial={{ opacity: 0, y: -2 }}
                    animate={{ opacity: 1, y: 0 }}
                    className="flex items-baseline gap-3 rounded-xl border border-line bg-bg-soft px-3 py-2"
                  >
                    <span className="tabular w-24 shrink-0 text-sm text-fg-dim">
                      {clock(entry.start_epoch)}
                      {entry.duration_s ? (
                        <span className="text-fg-faint"> · {length(entry.duration_s)}</span>
                      ) : null}
                    </span>

                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">{entry.summary}</span>
                      <span className="block truncate text-[12px] text-fg-faint">
                        {[
                          entry.location,
                          entry.attendees.length > 0
                            ? t("agenda.attendees", { count: entry.attendees.length })
                            : null,
                          entry.repeats ? t("agenda.repeats") : null,
                          entry.calendar,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>

                    {entry.conference && (
                      <a
                        href={entry.conference}
                        target="_blank"
                        rel="noreferrer noopener"
                        className="shrink-0 rounded-full bg-accent px-3 py-1 text-[13px] font-medium text-accent-fg hover:brightness-110"
                      >
                        {service(entry.conference)}
                      </a>
                    )}
                  </motion.li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}
