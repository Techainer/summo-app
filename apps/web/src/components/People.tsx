import { useCallback, useEffect, useState } from "react";

import { useT } from "../i18n/context";
import { useErrorText } from "../lib/errors";
import {
  PeopleClient,
  confidenceLabel,
  correctionSummary,
  nameOptions,
  speakingTime,
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
  const t = useT();
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
  }, [client, meeting]);

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
        setNotice(correctionSummary(correction));
        setError(null);
        await refresh();
      } catch (e) {
        setError(say(e));
      } finally {
        setBusy(null);
      }
    },
    [client, meeting, refresh],
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
    [client, draft, refresh],
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
    [client, refresh],
  );

  return (
    <section className="people">
      {error && <div className="banner error">{error}</div>}
      {notice && (
        <div className="banner info">
          {notice}
          <button type="button" className="link" onClick={() => setNotice(null)}>
            {t("common.close")}
          </button>
        </div>
      )}

      {voices.length > 0 && (
        <>
          <h2>{t("people.unknown")}</h2>
          <ul className="voice-list">
            {voices.map((voice) => (
              <li key={voice.label} className="voice">
                <div className="voice-head">
                  <strong>{voice.label}</strong>
                  <span className="muted">
                    {speakingTime(voice.seconds)} · {t("people.utterances", { count: voice.utterances })}
                  </span>
                </div>

                {voice.suggestions.length > 0 && (
                  <p className="muted suggestion-hint">
                    {t("people.maybe")}{" "}
                    {voice.suggestions.map((s, i) => (
                      <span key={s.id}>
                        {i > 0 && ", "}
                        <strong>{s.name}</strong> ({confidenceLabel(s.similarity)})
                      </span>
                    ))}
                  </p>
                )}

                <div className="voice-actions">
                  {nameOptions(voice, people).map((person) => (
                    <button
                      key={person.id}
                      type="button"
                      disabled={busy === voice.label}
                      onClick={() => void name(voice.label, person.name)}
                    >
                      {person.name}
                    </button>
                  ))}
                  <form
                    className="new-person"
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
                    />
                    <button type="submit" disabled={busy === voice.label}>
                      {t("common.save")}
                    </button>
                  </form>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      <h2>{t("people.known")}</h2>
      {space && <p className="muted">{t("people.identified_by", { space })}</p>}

      {people.length === 0 ? (
        <p className="empty">
          {t("people.empty")}
        </p>
      ) : (
        <ul className="person-list">
          {people.map((person) => (
            <li key={person.id} className="person">
              <span className="avatar" aria-hidden="true">
                {person.name.slice(0, 1)}
              </span>
              <div className="person-body">
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
                    />
                  </form>
                ) : (
                  <button
                    type="button"
                    className="link name"
                    onClick={() => {
                      setRenaming(person.id);
                      setDraft(person.name);
                    }}
                  >
                    {person.name}
                  </button>
                )}
                <span className="muted">
                  {t("people.samples", { count: person.samples })}
                  {person.confirmed > 0 && ` · ${t("people.confirmed_by_you", { count: person.confirmed })}`}
                  {person.centroids > 1 && ` · ${t("people.voice_styles", { count: person.centroids })}`}
                </span>
              </div>
              <button
                type="button"
                className="icon-button"
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
