import { useCallback, useEffect, useState } from "react";

import { useI18n } from "../i18n/context";
import { formatDuration } from "../lib/duration";
import { useErrorText } from "../lib/errors";
import {
  PeopleClient,
  confidenceLabel,
  correctionSummary,
  nameOptions,
  type Person,
  type UnknownVoice,
} from "../lib/people";

interface Props {
  client: PeopleClient;
  /** The meeting whose unnamed voices to ask about, if the user is looking at one. */
  meeting?: string;
}

/**
 * Who Summo can recognise, and naming the voices it could not.
 *
 * Two halves, in the order the work happens: the questions first — voices in this meeting that
 * still have no name — then the people already known. Putting the list first would bury the only
 * thing on the screen that needs the user to do something.
 */
export function People({ client, meeting }: Props) {
  const { t, locale } = useI18n();
  const say = useErrorText();
  const [people, setPeople] = useState<Person[]>([]);
  const [space, setSpace] = useState<string | undefined>();
  const [voices, setVoices] = useState<UnknownVoice[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");

  const refresh = useCallback(async () => {
    try {
      const view = await client.list();
      setPeople(view.people);
      setSpace(view.space);
      setVoices(meeting ? await client.unknowns(meeting) : []);
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, meeting, say]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const name = useCallback(
    async (label: string, personName: string) => {
      if (!meeting || !personName.trim()) return;
      setBusy(label);
      try {
        const correction = await client.nameVoice(meeting, label, personName.trim());
        // The daemon may have rewritten past meetings. Saying so is not optional.
        setNotice(
          correctionSummary(correction)
            .map((phrase) => t(phrase.key, phrase.params))
            .join(", ") || null,
        );
        setError(null);
        await refresh();
      } catch (e) {
        setError(say(e));
      } finally {
        setBusy(null);
      }
    },
    [client, meeting, refresh, say, t],
  );

  const commitRename = useCallback(
    async (id: string) => {
      if (!draft.trim()) {
        setRenaming(null);
        return;
      }
      try {
        await client.rename(id, draft.trim());
        setRenaming(null);
        setError(null);
        await refresh();
      } catch (e) {
        setError(say(e));
      }
    },
    [client, draft, refresh, say],
  );

  const forget = useCallback(
    async (person: Person) => {
      // Destructive and not obvious: the transcripts keep the name, the recognition does not.
      const ok = window.confirm(
        t("people.forget_confirm", { name: person.name }) + "\n\n" + t("people.forget_note"),
      );
      if (!ok) return;
      try {
        await client.forget(person.id);
        await refresh();
      } catch (e) {
        setError(say(e));
      }
    },
    [client, refresh, say, t],
  );

  return (
    <section className="mx-auto max-w-3xl space-y-4 p-6">
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec rounded-lg border px-3 py-2 text-[13px]">
          {error}
        </p>
      )}
      {notice && (
        <p className="border-accent/30 bg-accent-soft flex items-center gap-2 rounded-lg border px-3 py-2 text-[13px]">
          {notice}
          <button
            type="button"
            className="text-accent ml-auto shrink-0 hover:underline"
            onClick={() => setNotice(null)}
          >
            {t("common.close")}
          </button>
        </p>
      )}

      {voices.length > 0 && (
        <>
          <h2 className="text-xl font-semibold tracking-tight">{t("people.unknown")}</h2>
          <ul className="space-y-2.5">
            {voices.map((voice) => (
              <li key={voice.label} className="rounded-card border-line bg-bg-soft border p-3.5">
                <div className="flex items-baseline gap-2.5">
                  <strong className="text-[15px]">{voice.label}</strong>
                  <span className="text-fg-dim text-[12px]">
                    {formatDuration(voice.seconds, locale)} ·{" "}
                    {t("people.utterances", { count: voice.utterances })}
                  </span>
                </div>

                {voice.suggestions.length > 0 && (
                  <p className="text-fg-dim mt-1.5 text-[12px] leading-normal">
                    {t("people.maybe")}{" "}
                    {voice.suggestions.map((s, i) => (
                      <span key={s.id}>
                        {i > 0 && ", "}
                        <strong>{s.name}</strong> ({t(confidenceLabel(s.similarity))})
                      </span>
                    ))}
                  </p>
                )}

                {/* Wraps rather than scrolls: the list of colleagues is short, and a hidden name is an
                    unusable name. */}
                <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
                  {nameOptions(voice, people).map((person) => (
                    <button
                      key={person.id}
                      type="button"
                      disabled={busy === voice.label}
                      onClick={() => void name(voice.label, person.name)}
                      className="border-line bg-bg hover:border-accent hover:text-accent rounded-full border px-2.5 py-1 text-[13px] transition-colors disabled:cursor-default disabled:opacity-50"
                    >
                      {person.name}
                    </button>
                  ))}
                  <form
                    className="flex gap-1.5"
                    onSubmit={(e) => {
                      e.preventDefault();
                      const input = new FormData(e.currentTarget).get("name");
                      if (typeof input === "string") {
                        void name(voice.label, input);
                        e.currentTarget.reset();
                      }
                    }}
                  >
                    <input
                      name="name"
                      type="text"
                      placeholder={t("people.new_name")}
                      aria-label={t("people.name_this", { label: voice.label })}
                      disabled={busy === voice.label}
                      className="border-line bg-bg focus-visible:border-accent w-36 rounded-full border px-2.5 py-1 text-[13px] focus:outline-none"
                    />
                    <button
                      type="submit"
                      disabled={busy === voice.label}
                      className="border-line bg-bg hover:border-accent hover:text-accent rounded-full border px-2.5 py-1 text-[13px] disabled:opacity-50"
                    >
                      {t("common.save")}
                    </button>
                  </form>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      <h2 className="pt-2 text-xl font-semibold tracking-tight">{t("people.known")}</h2>
      {space && (
        <p className="text-fg-dim -mt-2 text-[12px]">{t("people.identified_by", { space })}</p>
      )}

      {people.length === 0 ? (
        <p className="text-fg-faint mt-16 text-center">{t("people.empty")}</p>
      ) : (
        <ul className="divide-line divide-y">
          {people.map((person) => (
            <li key={person.id} className="flex items-center gap-3 py-2.5">
              <span
                aria-hidden="true"
                className="border-line bg-bg-soft text-fg-dim grid h-9 w-9 shrink-0 place-items-center rounded-full border text-[15px] font-medium"
              >
                {person.name.slice(0, 1)}
              </span>
              <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                {renaming === person.id ? (
                  <form
                    onSubmit={(e) => {
                      e.preventDefault();
                      void commitRename(person.id);
                    }}
                  >
                    <input
                      type="text"
                      value={draft}
                      autoFocus
                      aria-label={t("people.rename_who", { name: person.name })}
                      onChange={(e) => setDraft(e.target.value)}
                      onBlur={() => void commitRename(person.id)}
                      className="border-accent bg-bg w-full rounded-md border px-2 py-0.5 text-sm focus:outline-none"
                    />
                  </form>
                ) : (
                  <button
                    type="button"
                    className="text-accent text-left text-sm font-medium hover:underline"
                    onClick={() => {
                      setRenaming(person.id);
                      setDraft(person.name);
                    }}
                  >
                    {person.name}
                  </button>
                )}
                <span className="text-fg-dim text-[12px]">
                  {t("people.samples", { count: person.samples })}
                  {person.confirmed > 0 &&
                    ` · ${t("people.confirmed_by_you", { count: person.confirmed })}`}
                  {person.centroids > 1 &&
                    ` · ${t("people.voice_styles", { count: person.centroids })}`}
                </span>
              </div>
              <button
                type="button"
                className="border-line bg-bg-soft text-fg-dim hover:border-rec hover:text-rec grid h-8 w-8 shrink-0 place-items-center rounded-lg border transition-colors"
                aria-label={t("people.forget_who", { name: person.name })}
                title={t("people.remove")}
                onClick={() => void forget(person)}
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
