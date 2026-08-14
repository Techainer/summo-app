import { RefreshCw } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { pickFile } from "../../lib/imports";
import { AgendaClient, type Calendars as CalendarList } from "../../lib/notes";
import { useLoad } from "../../lib/use-load";
import { Button, Input, Labelled } from "../ui";

/**
 * The calendars this app reads, and where they come from.
 *
 * Adding one used to mean typing the path of a `.ics` file, which is a snapshot: the agenda
 * describes whatever the calendar looked like on the day it was exported, and quietly keeps
 * describing it. Everything people actually use publishes a URL instead — Google calls it the
 * *secret address in iCal format*, Apple calls it a *public calendar* — and a URL the daemon can
 * fetch is a calendar that stays right.
 *
 * No account is connected. Signing in with Google would mean shipping a client secret inside an
 * open-source binary and asking for an account-wide scope so a notes app can learn when the standup
 * is; a link the user chooses to paste grants one calendar and is revocable from the calendar's own
 * settings. The instructions for finding it are on this screen, because that is the only hard part.
 *
 * Adding a file still works, and such a calendar is listed as what it is: something that will not
 * refresh.
 */
export function CalendarSources({ onChange }: { onChange: () => void }) {
  const { handshake } = useEngine();
  const { t, locale } = useI18n();
  const say = useErrorText();
  // Memoised because it is a dependency of the load below, and a new client every render would
  // re-fetch the calendar list on every keystroke in the address field.
  const client = useMemo(() => new AgendaClient(handshake), [handshake]);

  const [title, setTitle] = useState("");
  const [address, setAddress] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const list = useLoad(
    useCallback(async () => client.calendars(), [client]),
    [client],
  );

  // Both, always: the list of calendars and the agenda drawn from them are two views of the same
  // fetch, and refreshing one without the other is how a calendar appears with no meetings in it.
  const reload = () => {
    list.reload();
    onChange();
  };

  const subscribe = async () => {
    if (!address.trim()) return;
    setBusy(true);
    setError(null);
    try {
      // The name is optional: a calendar with no name is far likelier than one nobody can identify,
      // and the host is a better guess than an empty row.
      await client.subscribe(title.trim() || hostOf(address), address.trim());
      setTitle("");
      setAddress("");
      reload();
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const addFile = async () => {
    const chosen = await pickFile("iCalendar");
    if (chosen === null) {
      setError(t("import.no_dialog"));
      return;
    }
    if (!chosen.trim()) return;
    setError(null);
    try {
      const fallback = (chosen.split(/[/\\]/).pop() ?? "calendar").replace(/\.ics$/i, "");
      await client.addCalendar(chosen, title.trim() || fallback);
      setTitle("");
      reload();
    } catch (e) {
      setError(say(e));
    }
  };

  const refresh = async (name?: string) => {
    setBusy(true);
    setError(null);
    try {
      await client.refreshCalendars(name);
      reload();
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async (name: string) => {
    setError(null);
    try {
      await client.removeCalendar(name);
      reload();
    } catch (e) {
      setError(say(e));
    }
  };

  const calendars: CalendarList = list.data ?? { subscriptions: [], files: [] };

  return (
    <section className="border-line bg-bg-soft mt-5 rounded-[var(--radius-panel)] border p-4">
      <div className="flex flex-wrap items-end gap-2">
        <Labelled label={t("agenda.subscribe_label")} className="min-w-[16rem] flex-1">
          <Input
            value={address}
            onChange={(e) => setAddress(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void subscribe();
            }}
            placeholder={t("agenda.subscribe_placeholder")}
            // Never `type="url"`: a browser refuses to submit `webcal://…` as a URL, which is
            // precisely the address Apple and Outlook hand out.
            inputMode="url"
            spellCheck={false}
          />
        </Labelled>
        <Labelled label={t("agenda.name_label")} className="w-40">
          <Input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder={t("agenda.name_placeholder")}
          />
        </Labelled>
        <Button onClick={() => void subscribe()} disabled={busy || !address.trim()}>
          {t("agenda.subscribe")}
        </Button>
        <Button variant="ghost" onClick={() => void addFile()} disabled={busy}>
          {t("agenda.add_file")}
        </Button>
      </div>

      <details className="text-meta text-fg-faint mt-3">
        <summary className="cursor-pointer">{t("agenda.how")}</summary>
        <ul className="mt-2 space-y-1 ps-4">
          <li className="list-disc">{t("agenda.how_google")}</li>
          <li className="list-disc">{t("agenda.how_apple")}</li>
          <li className="list-disc">{t("agenda.how_outlook")}</li>
        </ul>
      </details>

      {error && (
        <p role="alert" className="text-danger mt-3 text-sm">
          {error}
        </p>
      )}

      {(calendars.subscriptions.length > 0 || calendars.files.length > 0) && (
        <ul className="mt-4 space-y-2" data-testid="calendar-list">
          {calendars.subscriptions.map((subscription) => (
            <li
              key={subscription.name}
              className="border-line bg-bg flex flex-wrap items-center gap-2 rounded-[var(--radius-card)] border px-3 py-2"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">{subscription.title}</span>
                <span
                  className={`text-micro block truncate ${
                    subscription.last_error ? "text-danger" : "text-fg-faint"
                  }`}
                >
                  {subscription.last_error ??
                    [
                      t("agenda.events", { count: subscription.events }),
                      subscription.last_sync === null
                        ? t("agenda.never_synced")
                        : t("agenda.synced", { when: when(subscription.last_sync, locale) }),
                    ].join(" · ")}
                </span>
              </span>
              <button
                type="button"
                aria-label={t("agenda.refresh_one", { name: subscription.title })}
                onClick={() => void refresh(subscription.name)}
                disabled={busy}
                className="text-fg-faint hover:text-fg"
              >
                <RefreshCw className={`size-4 ${busy ? "animate-spin" : ""}`} />
              </button>
              <button
                type="button"
                aria-label={t("agenda.remove_calendar", { name: subscription.title })}
                onClick={() => void remove(subscription.name)}
                className="text-fg-faint hover:text-danger"
              >
                ✕
              </button>
            </li>
          ))}

          {calendars.files.map((file) => (
            <li
              key={file.name}
              className="border-line bg-bg flex flex-wrap items-center gap-2 rounded-[var(--radius-card)] border px-3 py-2"
            >
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">{file.name}</span>
                {/* Said plainly. A file calendar that stopped matching reality is otherwise
                    indistinguishable from a subscription that is working. */}
                <span className="text-fg-faint text-micro block truncate">
                  {[t("agenda.events", { count: file.events }), t("agenda.from_file")].join(" · ")}
                </span>
              </span>
              <button
                type="button"
                aria-label={t("agenda.remove_calendar", { name: file.name })}
                onClick={() => void remove(file.name)}
                className="text-fg-faint hover:text-danger"
              >
                ✕
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** A host to name a calendar after, when the user did not name it. */
function hostOf(address: string): string {
  const withoutScheme = address.trim().replace(/^[a-z]+:\/\//i, "");
  return withoutScheme.split("/")[0] || "calendar";
}

/** "3 phút trước", from a timestamp, in whatever language the interface is in. */
function when(epoch: number, locale: string): string {
  const seconds = Math.round(epoch - Date.now() / 1000);
  const format = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return format.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return format.format(hours, "hour");
  return format.format(Math.round(hours / 24), "day");
}
