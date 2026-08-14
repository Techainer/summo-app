import { CalendarDays } from "lucide-react";
import { motion } from "motion/react";
import { useCallback, useMemo, useState } from "react";

import { Avatar, Empty, Page, PageGlow } from "../components/ui";
import { CalendarSources } from "../components/agenda/Calendars";
import { useErrorText } from "../lib/errors";
import { useI18n } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { GENTLE, listItem } from "../lib/motion";
import { AgendaClient, byDay, clock, length, service, type AgendaEntry } from "../lib/notes";
import { useRefresh } from "../lib/use-load";

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
  const say = useErrorText();
  const { t } = useI18n();
  const client = useMemo(() => new AgendaClient(handshake), [handshake]);

  const [entries, setEntries] = useState<AgendaEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setEntries(await client.list());
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useRefresh(refresh);

  const grouped = byDay(entries).reverse();

  return (
    <Page fill title={t("agenda.title")} subtitle={t("agenda.hint")} width="narrow">
      <PageGlow />

      {error && (
        <p role="alert" className="text-danger mt-4 text-sm">
          {error}
        </p>
      )}

      <CalendarSources onChange={() => void refresh()} />

      {entries.length === 0 ? (
        // `full`, so an empty calendar centres itself in what is left of the pane instead of
        // hanging in the upper third with a screen-height of background under it.
        <Empty full icon={CalendarDays} title={t("empty.agenda")} hint={t("empty.agenda_hint")} />
      ) : (
        <div className="mt-8 min-h-0 flex-1 space-y-6 overflow-y-auto">
          {grouped.map(([day, items]) => (
            <section key={day}>
              <h2 className="text-fg-faint text-micro font-semibold tracking-wider uppercase">
                {day}
              </h2>
              <ul className="mt-2 space-y-1.5">
                {items.map((entry) => (
                  <motion.li
                    key={`${entry.calendar}:${entry.uid}:${entry.start_epoch}`}
                    variants={listItem}
                    initial="hidden"
                    animate="shown"
                    transition={GENTLE}
                    className="border-line bg-bg-soft flex items-center gap-3 rounded-[var(--radius-card)] border px-3 py-2.5 shadow-[var(--shadow-sm)]"
                  >
                    <span className="tabular text-fg-dim w-24 shrink-0 text-sm">
                      {clock(entry.start_epoch)}
                      {entry.duration_s ? (
                        <span className="text-fg-faint"> · {length(entry.duration_s)}</span>
                      ) : null}
                    </span>

                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">{entry.summary}</span>
                      <span className="text-fg-faint text-micro block truncate">
                        {[
                          entry.location,
                          entry.attendees.length > 0
                            ? t("agenda.attendees", {
                                count: entry.attendees.length,
                              })
                            : null,
                          entry.repeats ? t("agenda.repeats") : null,
                          entry.calendar,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>

                    {/* Who is coming, as a stack of discs. An attendee count answers "how many"
                        and nothing else; the names are already in the invitation. */}
                    {entry.attendees.length > 0 && (
                      <span className="flex shrink-0 -space-x-1.5">
                        {entry.attendees.slice(0, 4).map((who) => (
                          <Avatar
                            key={who}
                            name={person(who)}
                            size="sm"
                            className="ring-bg-soft ring-2"
                          />
                        ))}
                      </span>
                    )}

                    {entry.conference && (
                      <a
                        href={entry.conference}
                        target="_blank"
                        rel="noreferrer noopener"
                        className="bg-accent text-accent-fg text-meta shrink-0 rounded-full px-3 py-1 font-medium hover:brightness-110"
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
    </Page>
  );
}

/**
 * An attendee as a person's name.
 *
 * Calendar invitations carry addresses as often as names — `ngoc.tran@acme.vn` — and initialling
 * that raw gives every colleague at the same company the same letter. The local part, with its
 * separators opened out, is the closest thing to a name the invitation actually contains.
 */
function person(attendee: string): string {
  const local = attendee.split("@")[0] ?? attendee;
  return local.replace(/[._-]+/g, " ").trim() || attendee;
}
