import { AudioLines } from "lucide-react";
import { Link } from "@tanstack/react-router";
import { motion } from "motion/react";
import { useCallback, useState } from "react";

import { Avatar, Empty, Page, SectionTitle } from "./ui";
import { listItem, stagger } from "../lib/motion";
import { useI18n } from "../i18n/context";
import { formatDuration } from "../lib/duration";
import { useErrorText } from "../lib/errors";
import { useRefresh } from "../lib/use-load";
import {
  PeopleClient,
  confidenceLabel,
  correctionSummary,
  nameOptions,
  type MeetingUnknowns,
  type Person,
} from "../lib/people";

interface Props {
  client: PeopleClient;
}

/**
 * Who Summo can recognise, and naming the voices it could not.
 *
 * Two halves, in the order the work happens: the questions first — voices that still have no name —
 * then the people already known. Putting the list first would bury the only thing on the screen
 * that needs the user to do something.
 *
 * The questions used to be about *one meeting*, passed in as a prop. Nothing ever passed it. So the
 * half of this screen that does the work — the whole point of a voice book — has never rendered,
 * and the screen has only ever been a read-only list of people it already knows. It now asks about
 * the whole vault, which is also the right question: a voice you cannot place is a voice you go
 * looking for, not one you happen to be looking at.
 */
export function People({ client }: Props) {
  const { t, locale } = useI18n();
  const say = useErrorText();
  const [people, setPeople] = useState<Person[]>([]);
  const [space, setSpace] = useState<string | undefined>();
  const [asking, setAsking] = useState<MeetingUnknowns[]>([]);
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
      setAsking(await client.unnamed());
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, say]);

  useRefresh(refresh);

  const name = useCallback(
    async (meeting: string, label: string, personName: string) => {
      if (!personName.trim()) return;
      setBusy(`${meeting}/${label}`);
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
    [client, refresh, say, t],
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
    // `h-full` and a column, so the empty state can take the pane and sit in the middle of it.
    // Without a height to fill, `Empty full` centres itself inside its own content box, which is
    // two lines tall — and the result is an empty state pinned to the top of five hundred pixels
    // of nothing, which is what it looked like before.
    <Page title={t("people.title")} subtitle={t("people.subtitle")} width="narrow" fill>
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
          {error}
        </p>
      )}
      {notice && (
        <p className="border-accent/30 bg-accent-soft text-meta flex items-center gap-2 rounded-lg border px-3 py-2">
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

      {asking.length > 0 && (
        <section data-testid="unnamed-voices" className="flex flex-col gap-2.5">
          <SectionTitle>{t("people.unknown")}</SectionTitle>
          {/* Said before the user acts rather than only reported afterwards. A correction that
              silently rewrites eleven old transcripts is alarming; one you were told about is the
              reason you bothered. */}
          <p className="text-fg-dim text-meta -mt-1.5">{t("people.naming_note")}</p>
          {asking.map((group) => (
            <div key={group.meeting} className="flex flex-col gap-1.5">
              {/* Which conversation, and a way into it. `S2` on its own is not a question anybody
                  can answer — the person naming a voice needs to know what was being talked about,
                  and often needs to go and listen to a line of it. */}
              <p className="text-fg-dim text-meta flex flex-wrap items-baseline gap-1.5">
                <Link
                  to="/pages/$pageId"
                  params={{ pageId: group.meeting }}
                  className="text-accent font-medium hover:underline"
                >
                  {group.title}
                </Link>
                <span className="text-fg-faint text-micro nums">{group.day}</span>
              </p>
              <ul className="flex flex-col gap-2.5">
                {group.voices.map((voice) => {
                  const working = busy === `${group.meeting}/${voice.label}`;
                  return (
                    <li
                      key={voice.label}
                      className="rounded-card border-line bg-bg-soft border p-3.5"
                    >
                      <div className="flex items-baseline gap-2.5">
                        <strong className="text-body">{voice.label}</strong>
                        <span className="text-fg-dim text-micro">
                          {formatDuration(voice.seconds, locale)} ·{" "}
                          {t("people.utterances", { count: voice.utterances })}
                        </span>
                      </div>

                      {voice.suggestions.length > 0 && (
                        <p className="text-fg-dim text-micro mt-1.5 leading-normal">
                          {t("people.maybe")}{" "}
                          {voice.suggestions.map((s, i) => (
                            <span key={s.id}>
                              {i > 0 && ", "}
                              <strong>{s.name}</strong> ({t(confidenceLabel(s.similarity))})
                            </span>
                          ))}
                        </p>
                      )}

                      {/* Wraps rather than scrolls: the list of colleagues is short, and a hidden
                          name is an unusable name. */}
                      <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
                        {nameOptions(voice, people).map((person) => (
                          <button
                            key={person.id}
                            type="button"
                            disabled={working}
                            onClick={() => void name(group.meeting, voice.label, person.name)}
                            className="border-line bg-bg hover:border-accent hover:text-accent text-meta rounded-full border px-2.5 py-1 transition-colors disabled:cursor-default disabled:opacity-50"
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
                              void name(group.meeting, voice.label, input);
                              e.currentTarget.reset();
                            }
                          }}
                        >
                          <input
                            name="name"
                            type="text"
                            placeholder={t("people.new_name")}
                            aria-label={t("people.name_this", { label: voice.label })}
                            disabled={working}
                            className="border-line bg-bg focus-visible:border-accent text-meta w-36 rounded-full border px-2.5 py-1 focus:outline-none"
                          />
                          <button
                            type="submit"
                            disabled={working}
                            className="border-line bg-bg hover:border-accent hover:text-accent text-meta rounded-full border px-2.5 py-1 disabled:opacity-50"
                          >
                            {t("common.save")}
                          </button>
                        </form>
                      </div>
                    </li>
                  );
                })}
              </ul>
            </div>
          ))}
        </section>
      )}

      {/* Only over a list. A heading above nothing is a heading that promises content the screen
          does not have, and on a new vault this one sat alone at the top of an empty pane with the
          "no voices yet" message four hundred pixels below it — two separate answers to the same
          question, neither next to the other. */}
      {people.length > 0 && (
        <>
          <SectionTitle>{t("people.known")}</SectionTitle>
          {space && (
            <p className="text-fg-dim text-micro -mt-2">{t("people.identified_by", { space })}</p>
          )}
        </>
      )}

      {people.length === 0 ? (
        // Only when there is nothing on the screen at all. With voices waiting to be named there is
        // work here, and `full` would centre "no voices yet" in the pane *below* it — a screen
        // simultaneously asking a question and claiming to be empty.
        asking.length === 0 && (
          <Empty full icon={AudioLines} title={t("people.empty_title")} hint={t("people.empty")} />
        )
      ) : (
        // A card each rather than rows separated by a hairline. The voice book is the one screen
        // where the unit of interest is a *person*, and a person is worth a surface.
        <motion.ul
          initial="hidden"
          animate="shown"
          transition={stagger(people.length)}
          className="grid gap-2 sm:grid-cols-2"
        >
          {people.map((person) => (
            <motion.li
              key={person.id}
              variants={listItem}
              className="border-line bg-bg-soft flex items-center gap-3 rounded-[var(--radius-card)] border p-3 shadow-[var(--shadow-sm)]"
            >
              <Avatar name={person.name} />
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
                <span className="text-fg-dim text-micro">
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
            </motion.li>
          ))}
        </motion.ul>
      )}
    </Page>
  );
}
