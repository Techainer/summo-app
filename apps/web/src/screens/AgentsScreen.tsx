import { AnimatePresence, motion } from "motion/react";
import { useCallback, useMemo, useRef, useState } from "react";

import {
  Avatar,
  Button,
  Card,
  CardBody,
  CardHeader,
  Page,
  PageGlow,
  StatusChip,
} from "../components/ui";
import { cn } from "../lib/cn";
import { useEngine } from "../lib/engine-context";
import { useErrorText } from "../lib/errors";
import { useI18n } from "../i18n/context";
import { GENTLE, listItem, screen as screenVariants, stagger } from "../lib/motion";
import { url } from "../lib/library";
import { useRefresh } from "../lib/use-load";

/**
 * The agents, and the files behind them.
 *
 * The screen deliberately does not hide that an agent is a document. There is no form with a field
 * per setting: the frontmatter carries a handful of settings a form could render, but the brief
 * below it is prose describing what the agent is for, and a form that could only express what the
 * form knows about would make the file the lesser thing — and then "just edit the file" would stop
 * being true the moment somebody wanted something the form had not anticipated.
 *
 * What the screen adds over a text editor is the part the file cannot show: which tools this agent
 * actually ends up with once the base's grant is applied, what it has learned, and whether its
 * `spawns` list names anybody real.
 */

interface Fact {
  learned: string;
  text: string;
}

interface Agent {
  slug: string;
  name: string;
  description: string;
  brief: string;
  provider: string | null;
  model: string | null;
  spawns: string[];
  /** Resolved: what it can call, not what its own file happens to list. */
  tools: string[];
  memory: Fact[];
  open_tasks: number;
}

interface Roster {
  agents: Agent[];
  base: string;
  base_tools: string[];
  /** `[agent, name-it-points-at]` for a `spawns` entry that names nobody. */
  dangling: [string, string][];
  skipped: { path: string; reason: string }[];
}

export function AgentsScreen() {
  const { t, n } = useI18n();
  const say = useErrorText();
  const { handshake } = useEngine();

  const [roster, setRoster] = useState<Roster | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  /** Whether the first agent has been opened for us. Saving reloads the roster, and reopening the
      first agent then would throw away whichever one the user was editing. */
  const picked = useRef(false);
  const [draft, setDraft] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  const open = useCallback(
    async (slug: string) => {
      setChosen(slug);
      setDraft(null);
      setSaved(null);
      try {
        const response = await fetch(url(handshake, `/agents/${slug}`));
        const body = (await response.json()) as {
          definition?: string;
          error?: string;
        };
        if (!response.ok) throw new Error(body.error ?? response.statusText);
        setDraft(body.definition ?? "");
      } catch (e) {
        setError(say(e));
      }
    },
    [handshake, say],
  );

  const load = useCallback(async () => {
    try {
      const response = await fetch(url(handshake, "/agents"));
      const body = (await response.json()) as Roster & { error?: string };
      if (!response.ok) throw new Error(body.error ?? response.statusText);
      setRoster(body);
      setError(null);
      // Open the first agent as it arrives, rather than showing three cards and a sentence asking
      // the user to pick one over half a window of nothing. Picking is not a decision worth making
      // somebody perform when the coordinator is the answer nine times out of ten, and the
      // definition file is what the screen is for. Done here rather than in an effect watching the
      // roster: an effect that sets state from state is a render cascade waiting to happen.
      const first = body.agents?.[0];
      if (first && !picked.current) {
        picked.current = true;
        void open(first.slug);
      }
    } catch (e) {
      setError(say(e));
    }
  }, [handshake, open, say]);

  useRefresh(load);

  const agent = useMemo(
    () => roster?.agents.find((a) => a.slug === chosen) ?? null,
    [roster, chosen],
  );

  const save = useCallback(async () => {
    if (!chosen || draft === null) return;
    setSaving(true);
    try {
      const response = await fetch(url(handshake, `/agents/${chosen}`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ definition: draft }),
      });
      const body = (await response.json()) as { error?: string };
      if (!response.ok) throw new Error(body.error ?? response.statusText);
      setSaved(t("agents.saved"));
      setError(null);
      // The roster is what shows the *resolved* tools and the dangling links, and both can change
      // with an edit — so it is re-read rather than patched in place.
      await load();
    } catch (e) {
      setError(say(e));
    } finally {
      setSaving(false);
    }
  }, [chosen, draft, handshake, load, say, t]);

  return (
    <Page title={t("agents.title")} subtitle={t("agents.subtitle")}>
      <PageGlow />

      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {/* A `spawns` entry naming nobody is otherwise invisible until a run tries to delegate, at
          which point the failure reads as the model's fault. */}
      {roster?.dangling.map(([from, to]) => (
        <p
          key={`${from}-${to}`}
          className="border-blocked/30 bg-blocked-soft text-blocked text-meta rounded-lg border px-3 py-2"
        >
          {t("agents.dangling", { from, to })}
        </p>
      ))}

      {roster?.skipped.map((broken) => (
        <p
          key={broken.path}
          className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2"
        >
          {t("agents.unreadable", { path: broken.path, reason: broken.reason })}
        </p>
      ))}

      <motion.div
        initial="hidden"
        animate="shown"
        transition={stagger(roster?.agents.length ?? 0)}
        className="grid gap-3 sm:grid-cols-2"
      >
        {roster?.agents.map((each) => (
          <motion.button
            key={each.slug}
            variants={listItem}
            type="button"
            onClick={() => void open(each.slug)}
            className={cn(
              "rounded-card bg-bg-soft border p-4 text-left",
              "transition-all duration-200 hover:-translate-y-0.5 hover:shadow-[var(--shadow-card)]",
              each.slug === chosen
                ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]"
                : "border-line hover:border-fg-faint",
            )}
          >
            <div className="flex items-center gap-2">
              <Avatar name={each.name} size="sm" />
              <span className="font-semibold">{each.name}</span>
              {/* Being able to start other agents is the one capability worth seeing from the
                  list: it is the difference between a worker and something that fans out. */}
              {each.spawns.length > 0 && (
                <StatusChip status="running" label={t("agents.coordinates")} />
              )}
              <span className="tabular text-fg-faint text-micro ml-auto">{each.slug}</span>
            </div>

            <p className="text-fg-dim text-meta mt-1.5 line-clamp-2 leading-normal">
              {each.description || each.brief}
            </p>

            <p className="text-fg-faint text-micro mt-2.5 flex flex-wrap items-center gap-1.5">
              <span className="tabular">{n("agents.tool_count", each.tools.length)}</span>
              {each.memory.length > 0 && (
                <span className="tabular">· {n("agents.memory_count", each.memory.length)}</span>
              )}
              {each.open_tasks > 0 && (
                <span className="tabular">· {n("agents.task_count", each.open_tasks)}</span>
              )}
              {each.model && <span className="tabular">· {each.model}</span>}
            </p>
          </motion.button>
        ))}
      </motion.div>

      <AnimatePresence mode="wait">
        {agent && draft !== null && (
          <motion.div
            key={agent.slug}
            variants={screenVariants}
            initial="enter"
            animate="show"
            exit="leave"
            transition={GENTLE}
            className="space-y-3"
          >
            <Card>
              <CardHeader title={agent.name} count={`vault/agents/${agent.slug}/AGENT.md`} />
              <CardBody className="space-y-3">
                <p className="text-fg-faint text-meta">{t("agents.file_hint")}</p>

                {/* A textarea, for the reason at the top of this file. Monospaced because the
                    frontmatter is YAML and indentation is load-bearing. */}
                <textarea
                  value={draft}
                  onChange={(e) => {
                    setDraft(e.target.value);
                    setSaved(null);
                  }}
                  spellCheck={false}
                  aria-label={t("agents.definition")}
                  className="border-line bg-bg text-fg focus-visible:border-accent text-meta h-72 w-full resize-y rounded-lg border px-3 py-2 font-mono leading-relaxed focus:outline-none"
                />

                <div className="flex flex-wrap items-center gap-3">
                  <Button onClick={() => void save()} disabled={saving}>
                    {saving ? t("common.saving") : t("common.save")}
                  </Button>
                  {saved && <span className="text-accent text-meta">{saved}</span>}
                </div>
              </CardBody>
            </Card>

            <div className="grid gap-3 sm:grid-cols-2">
              <Card>
                <CardHeader title={t("agents.tools")} />
                <CardBody>
                  <p className="text-fg-faint text-meta">{t("agents.tools_hint")}</p>
                  <ul className="mt-2 flex flex-wrap gap-1.5">
                    {agent.tools.map((tool) => (
                      <li
                        key={tool}
                        className="border-line bg-bg text-fg-dim text-micro rounded-full border px-2.5 py-0.5 font-mono"
                      >
                        {tool}
                      </li>
                    ))}
                  </ul>
                </CardBody>
              </Card>

              <Card>
                <CardHeader
                  title={t("agents.memory")}
                  count={`vault/agents/${agent.slug}/MEMORY.md`}
                />
                <CardBody>
                  {agent.memory.length === 0 ? (
                    <p className="text-fg-faint text-meta">{t("agents.memory_empty")}</p>
                  ) : (
                    <ul className="text-meta space-y-1.5">
                      {agent.memory.map((fact) => (
                        <li key={fact.text} className="flex gap-2">
                          <span className="tabular text-fg-faint text-micro shrink-0">
                            {fact.learned || "—"}
                          </span>
                          <span className="text-fg-dim">{fact.text}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </CardBody>
              </Card>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Under the roster, not floating in the middle of the pane: it is an instruction about the
          cards above it, and centring it in the empty half below made it look like the page had
          two unrelated halves. */}
      {roster && !agent && <p className="text-fg-faint text-meta">{t("agents.pick")}</p>}
    </Page>
  );
}
