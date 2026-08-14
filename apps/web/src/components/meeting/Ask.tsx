import { Sparkles } from "lucide-react";
import { useCallback, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { askAgent, fetchHabits, type Habit } from "../../lib/ask";
import { useLoad } from "../../lib/use-load";
import { Button, Input } from "../ui";

/**
 * Ask for something, about this note.
 *
 * There was a panel here with four buttons — email, message, recap, actions — and three tones, and
 * it was the wrong shape for what it did. Writing the follow-up email is not a feature beside
 * recording and summarising; it is one of the things a person asks for, and what comes back is a
 * note like every other note. A fixed menu of four could only ever be wrong for the fifth thing.
 *
 * So: a sentence, in the user's own words, handed to the agent — and above it, the sentences they
 * have used before. That list is not a guess. It is `vault/agents/HABITS.md`, the instructions they
 * have typed more than once, offered back so the fourth report costs one click instead of one
 * paragraph of typing. The agent is given the same list, so the fourth report also *looks* like the
 * first three, which is the part people actually complain about.
 */
export function AskPanel({ meeting }: { meeting: string }) {
  const { handshake } = useEngine();
  const { t } = useI18n();
  const say = useErrorText();

  const [instruction, setInstruction] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const habits = useLoad(
    useCallback(async () => fetchHabits(handshake), [handshake]),
    [handshake],
  );

  const ask = async (text: string) => {
    const wanted = text.trim();
    if (!wanted) return;
    setBusy(true);
    setError(null);
    setDone(false);
    try {
      await askAgent(handshake, wanted, meeting);
      setDone(true);
      setInstruction("");
      // Asked once more is asked twice: re-read so a habit appears the moment it becomes one.
      habits.reload();
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const usual: Habit[] = habits.data ?? [];

  return (
    <section
      className="border-line bg-bg-soft rounded-[var(--radius-panel)] border p-4"
      data-testid="ask"
    >
      <div className="flex items-center gap-2">
        <Sparkles className="text-ai size-4 shrink-0" aria-hidden="true" />
        <h2 className="flex-1 text-sm font-semibold">{t("ask.title")}</h2>
      </div>
      <p className="text-fg-faint text-micro mt-1">{t("ask.hint")}</p>

      {usual.length > 0 && (
        <div className="mt-3">
          <span className="text-fg-faint text-micro">{t("ask.usual")}</span>
          <ul className="mt-1 flex flex-wrap gap-1.5" data-testid="ask-habits">
            {usual.slice(0, 4).map((habit) => (
              <li key={habit.instruction}>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void ask(habit.instruction)}
                  className="border-line bg-bg text-meta hover:border-accent rounded-full border px-2.5 py-1"
                >
                  {habit.instruction}
                  <span className="text-fg-faint">
                    {" · "}
                    {t("ask.times", { count: habit.times })}
                  </span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Input
          value={instruction}
          onChange={(e) => setInstruction(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void ask(instruction);
          }}
          placeholder={t("ask.placeholder")}
          className="min-w-[16rem] flex-1"
          data-testid="ask-input"
        />
        <Button onClick={() => void ask(instruction)} disabled={busy || !instruction.trim()}>
          {busy ? t("ask.working") : t("ask.run")}
        </Button>
      </div>

      {error && (
        <p role="alert" className="text-danger mt-3 text-sm">
          {error}
        </p>
      )}
      {done && !error && <p className="text-fg-dim text-meta mt-3">{t("ask.done")}</p>}
    </section>
  );
}
