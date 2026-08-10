import { useCallback, useEffect, useState } from "react";

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
      setError(e instanceof Error ? e.message : String(e));
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
        setError(e instanceof Error ? e.message : String(e));
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
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [client, draft, refresh],
  );

  const forget = useCallback(
    async (person: Person) => {
      // Destructive and not obvious: the transcripts keep the name, the recognition does not.
      const ok = window.confirm(
        `Xoá ${person.name} khỏi danh sách giọng nói?\n\n` +
          `Các biên bản cũ vẫn giữ tên. Summo sẽ không tự nhận ra giọng này nữa.`,
      );
      if (!ok) return;
      try {
        await client.forget(person.id);
        await refresh();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
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
            Đóng
          </button>
        </div>
      )}

      {voices.length > 0 && (
        <>
          <h2>Chưa biết là ai</h2>
          <ul className="voice-list">
            {voices.map((voice) => (
              <li key={voice.label} className="voice">
                <div className="voice-head">
                  <strong>{voice.label}</strong>
                  <span className="muted">
                    {speakingTime(voice.seconds)} · {voice.utterances} câu
                  </span>
                </div>

                {voice.suggestions.length > 0 && (
                  <p className="muted suggestion-hint">
                    Có thể là{" "}
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
                      placeholder="Tên người mới"
                      aria-label={`Đặt tên cho ${voice.label}`}
                      disabled={busy === voice.label}
                    />
                    <button type="submit" disabled={busy === voice.label}>
                      Lưu
                    </button>
                  </form>
                </div>
              </li>
            ))}
          </ul>
        </>
      )}

      <h2>Giọng đã biết</h2>
      {space && <p className="muted">Nhận diện bằng {space}</p>}

      {people.length === 0 ? (
        <p className="empty">
          Chưa có ai. Ghi một buổi họp, rồi đặt tên cho giọng nói — lần sau Summo tự nhận ra.
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
                      aria-label={`Đổi tên ${person.name}`}
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
                  {person.samples} mẫu giọng
                  {person.confirmed > 0 && ` · ${person.confirmed} do bạn xác nhận`}
                  {person.centroids > 1 && ` · ${person.centroids} kiểu giọng`}
                </span>
              </div>
              <button
                type="button"
                className="icon-button"
                aria-label={`Xoá ${person.name}`}
                title="Xoá khỏi danh sách giọng nói"
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
