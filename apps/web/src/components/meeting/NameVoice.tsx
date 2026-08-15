import { useT } from "../../i18n/context";
import { confidenceLabel, nameOptions, type Person, type UnknownVoice } from "../../lib/people";

/**
 * Saying who a voice is, where you realised it.
 *
 * The voice book asks "who is `S2`?" out of context, from a list. That is the right screen for
 * working through a backlog and the wrong one for the moment the question is actually answerable:
 * you know who `S2` is because you are reading what they said. Naming from the transcript is one
 * click at the point of recognition instead of a screen, a scroll and a label you have to hold in
 * your head on the way.
 *
 * Rendered inline, under the chip, rather than as a popover. The transcript is virtualised and
 * measures each row as it renders, so an expanding row is something the list already handles —
 * whereas anything floating would need to survive the row being recycled out from under it.
 */
export function NameVoice({
  voice,
  people,
  busy,
  onName,
  onCancel,
}: {
  voice: UnknownVoice;
  people: Person[];
  busy: boolean;
  onName: (name: string) => void;
  onCancel: () => void;
}) {
  const t = useT();

  return (
    <div
      data-testid="name-voice"
      className="border-line bg-bg-raised mt-1.5 rounded-[var(--radius-card)] border p-2.5"
    >
      <p className="text-fg-dim text-micro">
        {t("people.name_this", { label: voice.label })}
        {voice.suggestions.length > 0 && (
          <>
            {" · "}
            {t("people.maybe")}{" "}
            {voice.suggestions.map((s, i) => (
              <span key={s.id}>
                {i > 0 && ", "}
                <strong>{s.name}</strong> ({t(confidenceLabel(s.similarity))})
              </span>
            ))}
          </>
        )}
      </p>

      {/* Wraps rather than scrolls: the list of colleagues is short, and a hidden name is an
          unusable name. */}
      <div className="mt-2 flex flex-wrap items-center gap-1.5">
        {nameOptions(voice, people).map((person) => (
          <button
            key={person.id}
            type="button"
            disabled={busy}
            onClick={() => onName(person.name)}
            className="border-line bg-bg hover:border-accent hover:text-accent text-meta rounded-full border px-2.5 py-1 transition-colors disabled:cursor-default disabled:opacity-50"
          >
            {person.name}
          </button>
        ))}
        <form
          className="flex gap-1.5"
          onSubmit={(e) => {
            e.preventDefault();
            const typed = new FormData(e.currentTarget).get("name");
            if (typeof typed === "string") onName(typed);
          }}
        >
          <input
            name="name"
            type="text"
            placeholder={t("people.new_name")}
            aria-label={t("people.name_this", { label: voice.label })}
            disabled={busy}
            className="border-line bg-bg focus-visible:border-accent text-meta w-32 rounded-full border px-2.5 py-1 focus:outline-none"
          />
          <button
            type="submit"
            disabled={busy}
            className="border-line bg-bg hover:border-accent hover:text-accent text-meta rounded-full border px-2.5 py-1 disabled:opacity-50"
          >
            {t("common.save")}
          </button>
        </form>
        <button
          type="button"
          onClick={onCancel}
          className="text-fg-faint hover:text-fg text-meta ms-auto px-1"
        >
          {t("common.close")}
        </button>
      </div>

      {/* Said before the user acts rather than only reported afterwards. A correction that silently
          rewrites eleven old transcripts is alarming; one you were told about is the reason you
          bothered. */}
      <p className="text-fg-faint text-micro mt-2">{t("people.naming_note")}</p>
    </div>
  );
}
