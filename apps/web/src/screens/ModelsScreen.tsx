import { CloudOff, HardDriveDownload, Package, Search, Trash2 } from "lucide-react";
import { m } from "motion/react";
import { useCallback, useEffect, useMemo, useState } from "react";

import { useNavigate, useSearch } from "@tanstack/react-router";

import { Markdown } from "../components/page/Markdown";
import { Button, Empty, Page, PageGlow, Progress, SectionTitle, Sheet } from "../components/ui";
import { useI18n, useT } from "../i18n/context";
import { cn } from "../lib/cn";
import {
  CatalogueClient,
  byTask,
  installedBytes,
  matches,
  roleFor,
  size,
  tags,
  type CatalogueModel,
  type LanguageAccuracy,
  type Role,
  type Task,
} from "../lib/catalogue";

/**
 * A real-time factor as the thing a person wants to know: how much faster than the meeting.
 *
 * `0.06` is unreadable; "17× faster than real time" is the same fact and answers "will this keep
 * up". Below 1.0 it keeps up; at or above, it falls behind and the phrasing changes to say so.
 */
function speedLabel(rtf: number): { times: number; keepsUp: boolean } {
  const keepsUp = rtf > 0 && rtf < 1;
  return { times: rtf > 0 ? Math.round(1 / rtf) : 0, keepsUp };
}

/**
 * Accuracy for the reader's own language, when it was measured.
 *
 * The one number a person actually wants from this list, and picking it here rather than showing
 * the whole list on a card is what keeps the card a card. The full list is on the detail sheet.
 */
function accuracyFor(model: CatalogueModel, locale: string): LanguageAccuracy | undefined {
  const mine = locale.toLowerCase().split("-")[0];
  return model.accuracy?.find((each) => each.lang.toLowerCase() === mine);
}

import { useEngine } from "../lib/engine-context";
import { fetchPlan, type Plan } from "../lib/plan";
import { useErrorText } from "../lib/errors";
import { listItem, stagger } from "../lib/motion";
import { OnboardingClient, POLL_MS, isFinished, type Install } from "../lib/onboarding";
import { languageName } from "../lib/languages";
import { url } from "../lib/library";
import { useRefresh } from "../lib/use-load";

/**
 * How long the app waits for a model's page before giving up on it.
 *
 * The daemon fetches the upstream README to build that page, so this is really a deadline on the
 * registry — eight seconds is long enough for a slow mirror and short enough that a blocked one is
 * an answer rather than a spinner somebody watches.
 */
const PAGE_TIMEOUT_MS = 8000;

/**
 * Pages already fetched, kept for as long as the app is open.
 *
 * A model page is a manifest and an upstream README: it does not change while somebody is browsing,
 * and re-fetching it cost a round trip to the registry every time a card was opened — which on a
 * slow connection is the difference between "opens" and "thinks about it". Keyed by id *and*
 * language, because the daemon renders the prose in the language it is asked for.
 */
const PAGES = new Map<string, string>();

/**
 * Every model, what it is for, and a button.
 *
 * The registry has always been able to answer this and nothing ever asked it: the only way to
 * install a model that was not the recommended one was `summo pull` on a command line. A local-first
 * app whose whole promise is which models run on your machine has to let you see them.
 *
 * Three decisions:
 *
 * **Grouped by what the model does**, not by name or size. "Which speech model" and "which
 * translator" are different questions asked at different times, and a flat list makes both harder.
 *
 * **The facts that change a decision, on the card.** Size, licence, languages — and the two that
 * cost a user an afternoon if they find out at the download instead: `gated` means an account is
 * needed upstream, `upstream` means Summo does not host it.
 *
 * **An unreachable registry is a state, not an error.** The screen shows what is installed and says
 * the catalogue is offline. Going blank without a network would be worse, and this is an app that
 * is expected to work on a plane.
 */
export function ModelsScreen() {
  const t = useT();
  const { lang } = useSearch({ from: "/models" });
  const navigate = useNavigate();
  const { locale } = useI18n();
  const say = useErrorText();
  const { handshake } = useEngine();
  const catalogue = useMemo(() => new CatalogueClient(handshake), [handshake]);
  const installer = useMemo(() => new OnboardingClient(handshake), [handshake]);

  const [models, setModels] = useState<CatalogueModel[] | null>(null);
  const [reachable, setReachable] = useState(true);
  const [installs, setInstalls] = useState<Install[]>([]);
  /** Which model fills each role, so the card can say "in use" rather than offering a button. */
  const [chosen, setChosen] = useState<Record<string, string | null>>({});
  const [error, setError] = useState<string | null>(null);
  /** What was typed into the search box, and which task is being looked at. */
  const [query, setQuery] = useState("");
  const [task, setTask] = useState<Task | null>(null);
  /** What a recording would actually use right now, asked of the daemon rather than inferred. */
  const [plan, setPlan] = useState<Plan | null>(null);

  const load = useCallback(async () => {
    try {
      const [next, running, current] = await Promise.all([
        catalogue.load(),
        installer.installs(),
        fetchPlan(handshake).catch(() => null),
      ]);
      setPlan(current);
      setModels(next.models);
      setReachable(next.reachable);
      setInstalls(running);
      setChosen(next.chosen ?? {});
      setError(null);
    } catch (e) {
      // The daemon answers with `reachable: false` when the *registry* is unreachable, so getting
      // here means the daemon itself did not answer. Either way the honest screen is the same one:
      // what is installed, and a line saying the catalogue is not available. Showing a bare fetch
      // error instead would be technically accurate and useless.
      setModels((current) => current ?? []);
      setReachable(false);
      setError(say(e));
    }
  }, [catalogue, installer, handshake, say]);

  useRefresh(load);

  // While something is downloading, and only then. Polling an idle screen every second is a
  // request per second for a list that has not changed.
  const downloading = installs.some((job) => !isFinished(job));
  useEffect(() => {
    if (!downloading) return undefined;
    const timer = window.setInterval(() => void load(), POLL_MS);
    return () => window.clearInterval(timer);
  }, [downloading, load]);

  const remove = useCallback(
    async (id: string) => {
      try {
        await catalogue.remove(id);
        await load();
        setError(null);
      } catch (e) {
        // The daemon refuses to remove a model the settings point at, and says which role it
        // fills. That message is the whole answer, so it is shown rather than summarised.
        setError(say(e));
      }
    },
    [catalogue, load, say],
  );

  const choose = useCallback(
    async (model: CatalogueModel, want?: Role) => {
      const role = want ?? roleFor(model.task);
      if (!role) return;
      try {
        await catalogue.use(role, model.id);
        setChosen((current) => ({ ...current, [role]: model.id }));
        setError(null);
      } catch (e) {
        setError(say(e));
      }
    },
    [catalogue, say],
  );

  /// Put a role back to "nothing chosen".
  ///
  /// Only noise suppression uses this today, and it is the only role that needs it: every other one
  /// falls back to something sensible when unset, so choosing a different model is how you change
  /// your mind. An enhancer that is unset is *off*, so without a way back the first click is
  /// permanent — the user would have to edit the settings file to record without it again.
  const stop = useCallback(
    async (role: Role) => {
      try {
        await catalogue.use(role, "");
        setChosen((current) => ({ ...current, [role]: null }));
        setError(null);
      } catch (e) {
        setError(say(e));
      }
    },
    [catalogue, say],
  );

  const pull = useCallback(
    async (id: string) => {
      try {
        const job = await installer.install(id);
        setInstalls((current) => [...current.filter((i) => i.model !== id), job]);
      } catch (e) {
        setError(say(e));
      }
    },
    [installer, say],
  );

  if (!models) {
    return (
      <p className="text-fg-faint grid h-full place-items-center text-center">
        {error ?? t("common.loading")}
      </p>
    );
  }

  // Narrowed to one language when the app sent somebody here to solve a specific gap. `*` counts:
  // a multilingual model does serve Japanese, and hiding it would be a filter that lies.
  const wanted = lang?.toLowerCase();
  const forLanguage = wanted
    ? models.filter(
        (model) =>
          model.langs.length === 0 ||
          model.langs.some((code) => code === "*" || code.toLowerCase().split("-")[0] === wanted),
      )
    : models;
  // The task chips describe the shelf as it is, not as the interface imagines it: a registry that
  // starts publishing denoisers gets a chip for them without a release here.
  const offered = byTask(forLanguage).map((group) => group.task);
  const shown = forLanguage.filter(
    (model) => (task === null || model.task === task) && matches(model, query),
  );
  const groups = byTask(shown);

  return (
    <Page title={t("models.title")} subtitle={t("models.hint")} data-testid="models">
      <PageGlow />
      {/* What they cost, which is why somebody opens this screen a second time. */}
      {installedBytes(models) > 0 && (
        <p className="text-fg-dim nums text-micro -mt-4">
          {t("models.on_disk", { size: size(installedBytes(models)) })}
        </p>
      )}

      {!reachable && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta mt-4 flex items-center gap-2 rounded-lg border px-3 py-2">
          <CloudOff aria-hidden="true" className="size-4 shrink-0" />
          {t("models.offline")}
        </p>
      )}
      {error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta mt-4 rounded-lg border px-3 py-2">
          {error}
        </p>
      )}

      {/* Why this shelf is short. Somebody arrives here from a language they picked, and a
          catalogue that silently shows three of its eight models looks broken rather than
          filtered. */}
      {wanted && (
        <p className="border-line bg-bg-soft text-meta mb-3 flex flex-wrap items-center gap-2 rounded-[var(--radius-card)] border px-3 py-2">
          <span>{t("models.for_language", { language: languageName(wanted, locale) })}</span>
          <button
            type="button"
            onClick={() => void navigate({ to: "/models", search: {} })}
            className="text-accent font-medium underline"
          >
            {t("models.show_all")}
          </button>
        </p>
      )}

      {/* What is running, before what could be. "Which of these is actually being used" was the
          question this screen could not answer: a card said "Đang dùng" because the settings named
          it, whether or not the bytes were ever downloaded, and the two models a recording cannot
          start without appeared nowhere until they were missing. */}
      {plan && (
        <Running
          plan={plan}
          onInstall={(id) => void pull(id)}
          onSecond={(id) =>
            void catalogue
              .use("refine", id)
              .then(load)
              .catch((e) => setError(say(e)))
          }
          installs={installs}
        />
      )}

      {/* Search and a task filter, because the catalogue is now long enough to scroll past what you
          came for. Both narrow the same list the language banner narrows — one row of controls, not
          a filter panel: there are three axes worth filtering on and two of them fit on a line. */}
      <div className="mb-3 flex flex-wrap items-center gap-2">
        <div className="border-line bg-bg-soft focus-within:border-line-strong flex min-w-[12rem] flex-1 items-center gap-2 rounded-full border px-3 py-1.5 transition-colors">
          <Search aria-hidden="true" className="text-fg-faint size-3.5 shrink-0" />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("models.search")}
            aria-label={t("models.search")}
            data-testid="model-search"
            className="text-meta placeholder:text-fg-faint w-full bg-transparent outline-none"
          />
        </div>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip active={task === null} onClick={() => setTask(null)}>
            {t("models.all_tasks")}
          </Chip>
          {offered.map((each) => (
            <Chip key={each} active={task === each} onClick={() => setTask(each)}>
              {t(`models.task_${each.replace("-", "_")}`)}
            </Chip>
          ))}
        </div>
      </div>

      {groups.length === 0 ? (
        // Two different empty screens. A catalogue with nothing in it is a connection problem; a
        // catalogue with nothing *matching* is a typo, and telling somebody the registry is offline
        // when they have simply mistyped a name sends them to fix the wrong thing.
        query || task !== null ? (
          <Empty
            icon={Search}
            title={t("models.no_match")}
            hint={t("models.no_match_hint")}
            action={
              <Button
                size="sm"
                variant="secondary"
                onClick={() => {
                  setQuery("");
                  setTask(null);
                }}
              >
                {t("models.clear_filters")}
              </Button>
            }
          />
        ) : (
          <Empty icon={Package} title={t("models.none")} hint={t("models.offline")} />
        )
      ) : (
        groups.map((group) => (
          <section key={group.task}>
            <SectionTitle>{t(`models.task_${group.task.replace("-", "_")}`)}</SectionTitle>
            {/* Cards arrive rather than appear. A grid of eight that pops in at once reads as a
                page repainting; a short stagger reads as a list being laid out, and it gives the
                eye an order to follow. `stagger` shortens the step as the count grows, so a long
                section does not take a second to finish. */}
            <m.div
              initial="hidden"
              animate="shown"
              transition={stagger(group.models.length)}
              className="mt-2.5 grid gap-2.5 lg:grid-cols-2"
            >
              {group.models.map((model) => (
                <Card
                  key={model.id}
                  model={model}
                  job={installs.find((job) => job.model === model.id)}
                  onPull={() => void pull(model.id)}
                  onRemove={() => void remove(model.id)}
                  onUse={() => void choose(model)}
                  onRefine={() => void choose(model, "refine")}
                  onStop={() => {
                    const role = roleFor(model.task);
                    if (role) void stop(role);
                  }}
                  optional={model.task === "denoise"}
                  asRefine={chosen.refine === model.id && model.installed}
                  // Chosen *and* here. Naming a model in the settings does not make it usable, and
                  // a card reading "Đang dùng" over an Install button was the screen telling two
                  // opposite things at once.
                  inUse={chosen[roleFor(model.task) ?? ""] === model.id && model.installed}
                  picked={chosen[roleFor(model.task) ?? ""] === model.id}
                />
              ))}
            </m.div>
          </section>
        ))
      )}
    </Page>
  );
}

/**
 * What a recording would use, right now, on this machine.
 *
 * Four roles, and every one of them was invisible until it failed. Speech recognition was named on
 * a card that said "in use" whether or not the model had ever been downloaded; the voice detector
 * and the speaker embedder appeared nowhere at all, so "I pressed record and nothing happened" and
 * "it will not say who spoke" had no answer on the screen that exists to answer them.
 *
 * Read from `/settings/plan`, which is the daemon resolving the same question a session resolves
 * when it starts — not the interface guessing from the settings file.
 */
function Running({
  plan,
  installs,
  onInstall,
  onSecond,
}: {
  plan: Plan;
  installs: Install[];
  onInstall: (id: string) => void;
  onSecond: (id: string) => void;
}) {
  const t = useT();

  const rows: { key: string; label: string; value: string | null; ready: boolean; id?: string }[] =
    [
      {
        key: "asr",
        label: t("models.role_asr"),
        value: plan.speech.name ?? plan.speech.model,
        ready: plan.speech.installed,
        ...(plan.speech.model ? { id: plan.speech.model } : {}),
      },
      // Only when there is one. This role is genuinely optional — most meetings are in one language
      // and a second decode of every utterance would cost without adding — so an always-present row
      // reading "none" would teach people to ignore a row that matters when it is not empty.
      ...(plan.second_pass?.model
        ? [
            {
              key: "second",
              label: t("models.role_second"),
              value: plan.second_pass?.name ?? plan.second_pass?.model,
              ready: plan.second_pass?.installed === true,
              id: plan.second_pass?.model,
            },
          ]
        : []),
      {
        key: "vad",
        label: t("models.role_vad"),
        value: plan.detector.id,
        ready: plan.detector.installed,
        id: plan.detector.id,
      },
      {
        key: "speaker",
        label: t("models.role_speaker"),
        value: plan.speakers.id,
        ready: plan.speakers.installed,
        id: plan.speakers.id,
      },
      {
        key: "translate",
        label: t("models.role_translate"),
        // Translation is the one role that can be somebody else's server, and then there is nothing
        // to install and nothing to be missing.
        value: plan.translation.local
          ? plan.translation.model
          : plan.translation.provider
            ? t("models.role_endpoint", { provider: plan.translation.provider })
            : null,
        // An endpoint is always ready — there is nothing to install. A local model is ready when
        // it is on disk, which the daemon now reports rather than the interface assuming.
        ready: !plan.translation.local || plan.translation.installed,
        ...(plan.translation.local && plan.translation.model ? { id: plan.translation.model } : {}),
      },
    ];

  return (
    <section
      data-testid="running"
      className="border-line bg-bg-raised mb-4 rounded-[var(--radius-card)] border p-4 shadow-[var(--shadow-sm)]"
    >
      <div className="mb-2.5 flex items-baseline justify-between gap-3">
        <h2 className="text-meta font-semibold">{t("models.running_title")}</h2>
        <p className="text-fg-faint text-micro">{t("models.running_hint")}</p>
      </div>
      <ul className="grid gap-2 sm:grid-cols-2">
        {rows.map((row) => {
          const job = installs.find((each) => each.model === row.id);
          const running = job !== undefined && !isFinished(job);
          return (
            <li
              key={row.key}
              data-testid={`running-${row.key}`}
              className="border-line flex items-center gap-2.5 rounded-[var(--radius-card)] border px-3 py-2"
            >
              <span
                aria-hidden="true"
                className={cn("size-2 shrink-0 rounded-full", row.ready ? "bg-done" : "bg-blocked")}
              />
              <span className="text-micro text-fg-faint w-28 shrink-0">{row.label}</span>
              <span className="text-meta min-w-0 flex-1 truncate">
                {row.value ?? t("models.role_none")}
              </span>
              {!row.ready && row.id && (
                <Button
                  size="sm"
                  variant="secondary"
                  busy={running}
                  onClick={() => onInstall(row.id as string)}
                >
                  {t("models.install")}
                </Button>
              )}
              {!row.ready && !row.id && (
                <span className="text-blocked text-micro">{t("models.role_missing")}</span>
              )}
            </li>
          );
        })}
      </ul>

      {/* The pairing nothing has ever suggested.
          A meeting with two languages in it needs a model that hears the one the live model
          cannot, and that is a coverage question — `Recommendation::pair` asks an accuracy one and
          so answers "nothing worth adding" for precisely the meeting where the second model
          matters most. Offered, never applied: the app does not swap models on somebody's behalf,
          and this one costs a second decode of every utterance. */}
      {plan.second_pass?.suggested && (
        <div className="border-accent/30 bg-accent-soft text-meta mt-3 rounded-lg border px-3 py-2">
          <p className="text-fg-dim">
            <span className="text-fg font-medium">{plan.second_pass?.suggested.name}</span>{" "}
            {plan.second_pass?.suggested.reason}
          </p>
          <div className="mt-2">
            <Button
              size="sm"
              variant="secondary"
              disabled={!plan.second_pass?.suggested.installed}
              onClick={() => onSecond(plan.second_pass?.suggested?.id as string)}
            >
              {plan.second_pass?.suggested.installed
                ? t("models.use_refine")
                : t("models.role_missing")}
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/**
 * What was measured, in the three numbers a choice actually turns on.
 *
 * Accuracy, speed, memory — for *this* reader and *this* machine. Every one of them has been in the
 * manifests since the registry existed and none of them reached the screen, so choosing between
 * Gipformer and Whisper meant comparing two names and two file sizes. The file size is the one
 * thing that does not matter once both are installed.
 *
 * Three deliberate choices:
 *
 * **Accuracy is for the reader's language, not an average.** Whisper tiny is 95 % on English and
 * 32 % on Vietnamese. One number would recommend it to a Vietnamese speaker or hide why anybody
 * installs it, depending which one you picked. When this reader's language was never benchmarked
 * the cell is absent rather than empty — "not measured" is information, "—" is furniture.
 *
 * **Speed is "× faster than real time", not a real-time factor.** `0.06` is a number for a
 * benchmark table; "17× faster" answers "will this keep up with my meeting". Where the published
 * figures are for another class of machine — every Apple Silicon Mac today, since the benchmarks
 * are x86 — it says so rather than passing somebody else's hardware off as yours.
 *
 * **Nothing is shown that was not measured.** A model with no published benchmarks renders no row
 * at all. Zeros in these positions read as "this model is bad", which is a different claim from
 * "nobody has tested it".
 */
function Measured({ model }: { model: CatalogueModel }) {
  const { t, locale } = useI18n();
  const mine = accuracyFor(model, locale);
  const speed = model.speed ? speedLabel(model.speed.rtf) : null;

  const cells: { value: string; label: string; dim?: boolean }[] = [];
  if (mine) {
    cells.push({
      value: `${Math.round(mine.accuracy * 100)}%`,
      label: languageName(mine.lang, locale),
    });
  }
  if (speed && speed.times > 0) {
    cells.push({
      value: speed.keepsUp ? `${speed.times}×` : t("models.too_slow"),
      label: t("models.vs_realtime"),
      // Greyed when it is somebody else's hardware, and the detail sheet says whose.
      dim: model.speed?.measured_here === false,
    });
  }
  if (model.rss_peak_mb) {
    cells.push({ value: `${model.rss_peak_mb} MB`, label: t("models.while_running") });
  }
  if (cells.length === 0) return null;

  return (
    <dl className="border-line mt-3 grid grid-cols-3 gap-2 border-t pt-2.5">
      {cells.map((cell) => (
        <div key={cell.label}>
          <dt className="sr-only">{cell.label}</dt>
          <dd
            className={cn(
              "nums text-sm leading-none font-semibold tabular-nums",
              cell.dim ? "text-fg-faint" : "text-fg",
            )}
          >
            {cell.value}
          </dd>
          <p className="text-fg-faint text-micro mt-1 truncate leading-none">{cell.label}</p>
        </div>
      ))}
    </dl>
  );
}

/**
 * A card-sized version of a model's own description.
 *
 * The first sentence, capped. Registry descriptions run to a paragraph — SMALL100's is nine lines
 * about embedding tables and quantisation — and a grid of those is a wall nobody reads. Cut on a
 * sentence boundary rather than a character count, so what is shown is a whole thought.
 */
function summarise(description: string): string {
  const text = description.trim().replace(/\s+/g, " ");
  const stop = /(?<=[.!?])\s/.exec(text);
  const first = stop ? text.slice(0, stop.index + 1) : text;
  return first.length > 180 ? `${first.slice(0, 179).trimEnd()}…` : first;
}

/** One filter, pressed or not. A toggle, so it carries `aria-pressed` rather than looking like one. */
function Chip({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className={cn(
        "text-micro rounded-full border px-2.5 py-1 transition-colors",
        active
          ? "border-accent/50 bg-accent-soft text-accent"
          : "border-line text-fg-dim hover:border-line-strong hover:text-fg",
      )}
    >
      {children}
    </button>
  );
}

function Card({
  model,
  job,
  onPull,
  onRemove,
  onUse,
  onRefine,
  onStop,
  inUse,
  asRefine,
  picked,
  optional,
}: {
  model: CatalogueModel;
  job: Install | undefined;
  onPull: () => void;
  onRemove: () => void;
  onUse: () => void;
  onRefine: () => void;
  /** Put the role back to nothing chosen. Only shown for an `optional` model in use. */
  onStop: () => void;
  inUse: boolean;
  /** Whether this is the model that re-decodes after the live pass. */
  asRefine: boolean;
  /** Named in the settings, whether or not the files are on this machine. */
  picked: boolean;
  /**
   * Whether "not chosen" is a working state for this role.
   *
   * True only for noise suppression. Recording needs *a* speech model and *a* voice detector, so
   * those roles are never off and their cards only ever switch between models. An enhancer is off
   * by default and has to be able to go back — a card whose only control turns something on is one
   * where the first click is permanent.
   */
  optional: boolean;
}) {
  const t = useT();
  // Two clicks, not a dialog. Re-downloading a gigabyte is a real cost, and a modal for it would
  // be one more thing to dismiss on the screen where somebody is tidying up several models.
  const [confirming, setConfirming] = useState(false);
  const [open, setOpen] = useState(false);
  // Keyed by the language it was fetched in, so switching the interface to English re-fetches the
  // page in English rather than leaving Vietnamese prose under English headings.
  const { handshake } = useEngine();
  const { locale } = useI18n();
  // Starts from the cache when there is one, so a card opened a second time draws its page in the
  // same frame rather than fetching it again — and `useState` rather than an effect, because a
  // `setState` during an effect is a second render before the browser has painted the first.
  const [page, setPage] = useState<{ lang: string; markdown: string } | null>(() => {
    const cached = PAGES.get(`${model.id}:${locale}`);
    return cached === undefined ? null : { lang: locale, markdown: cached };
  });

  // Fetched when it is opened, not with the list: ten model pages, each carrying an upstream
  // README, is a lot of text to download for a screen most people scroll past.
  //
  // On a deadline, because the daemon fetches that README from the registry and a registry it
  // cannot reach costs it a full connect timeout. Opening the details on a blocked network showed
  // "Đang tải…" and kept showing it — the one state a spinner must never end in.
  useEffect(() => {
    if (!open || page?.lang === locale) return undefined;
    const stop = new AbortController();
    const deadline = window.setTimeout(() => stop.abort(), PAGE_TIMEOUT_MS);
    fetch(url(handshake, `/models/${encodeURIComponent(model.id)}/page`, { lang: locale }), {
      signal: stop.signal,
    })
      .then((r) => r.json())
      .then((body: { markdown?: string }) => {
        PAGES.set(`${model.id}:${locale}`, body.markdown ?? "");
        setPage({ lang: locale, markdown: body.markdown ?? "" });
      })
      .catch(() => setPage({ lang: locale, markdown: "" }))
      .finally(() => window.clearTimeout(deadline));
    return () => {
      window.clearTimeout(deadline);
      stop.abort();
    };
  }, [open, page, handshake, model.id, locale]);

  const running = job !== undefined && !isFinished(job);

  return (
    <m.article
      variants={listItem}
      className={cn(
        // A real card on the page surface, not a translucent tint of it. Elevation is what tells
        // the eye these are eight separate things to choose between.
        "border-line bg-bg-raised rounded-[var(--radius-card)] border p-4 shadow-[var(--shadow-sm)]",
        // Lifts under the pointer. The whole card is a decision — read it, install it, remove it —
        // so the whole card should acknowledge the cursor rather than only the button on it.
        "transition-[transform,box-shadow,border-color,background-color] duration-150",
        "hover:bg-bg-elevated hover:border-line-strong hover:-translate-y-0.5 hover:shadow-[var(--shadow-card)]",
        inUse && "border-accent/40",
      )}
    >
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <h3 className="text-body leading-snug font-medium">{model.name}</h3>
            {/* State on the title line, where the eye already is: whether this machine has it, and
                whether it is the one being used. Those were two buttons and a word in three
                different places on the card. */}
            {inUse ? (
              <span className="border-accent/40 text-accent text-micro rounded-full border px-2 py-0.5">
                {t("models.in_use")}
              </span>
            ) : picked ? (
              // Pointed at by the settings and not downloaded. This is the state that made the
              // screen contradict itself, and it is worth naming rather than hiding: it is why
              // translation is switched on and nothing gets translated.
              <span className="border-blocked/40 text-blocked text-micro rounded-full border px-2 py-0.5">
                {t("models.picked_not_installed")}
              </span>
            ) : model.installed ? (
              <span className="border-done/40 text-done text-micro rounded-full border px-2 py-0.5">
                {t("models.installed")}
              </span>
            ) : null}
          </div>
          <p className="text-fg-faint tabular text-micro mt-0.5">
            {model.id}
            {model.size_bytes > 0 && ` · ${size(model.size_bytes)}`}
          </p>
        </div>
      </div>

      {/* Clamped, and the registry's own text is not shortened to match.
          A manifest description is written for the model's page — measured numbers, licence
          reasoning, why one model beats another — which is right there and far too long here. One
          card ran to twelve lines beside a neighbour that ran to three, and a two-column grid of
          those is unreadable before a word of it is read. Three lines is enough to decide whether
          to keep reading. */}
      {/* The first sentence, and only the first.
          A manifest description is written for the model's page — measured numbers, licence
          reasoning, why one model beats another — and clamping it to three lines still put a
          paragraph of prose on a card whose job is to be scanned. The whole text is one click away
          in the details panel, where it belongs. */}
      {model.description && (
        <p className="text-fg-dim text-meta mt-2 line-clamp-2 leading-relaxed">
          {summarise(model.description)}
        </p>
      )}

      <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
        {tags(model, locale).map((tag) => (
          <span
            key={tag.label}
            className={cn(
              "text-micro rounded-full border px-2 py-0.5",
              tag.kind === "warn" && "border-blocked/40 text-blocked",
              tag.kind === "good" && "border-accent/40 text-accent",
              tag.kind === "plain" && "border-line text-fg-faint",
            )}
          >
            {tag.label}
          </span>
        ))}
      </div>

      {/* The three numbers somebody comparing two models is actually comparing.
          A name, a size and three chips is what this card had, and none of it answers "is it
          accurate", "is it fast enough" or "will it fit". All three are measured and published per
          model and were being dropped by `/catalogue`. */}
      <Measured model={model} />

      {/* Actions last, and on a line of their own.
          They used to sit beside the title, and for an installed speech model that is three
          buttons — use, use-for-refine, remove — competing with the name for one row. The name
          lost: "Gipformer 65M · Vietnamese" wrapped onto three lines next to a column of controls.
          A card reads identity, then facts, then what you can do about them. */}
      <div className="border-line mt-3 flex flex-wrap items-center gap-2 border-t pt-3">
        {model.installed ? (
          <div className="flex shrink-0 items-center gap-2">
            {/* Installed and *chosen* are different states, and conflating them is what made the
              catalogue decorative: a user could install a Japanese model and record in
              Vietnamese with no indication of why. */}
            {inUse
              ? optional && (
                  <Button size="sm" variant="ghost" onClick={onStop}>
                    {t("models.turn_off")}
                  </Button>
                )
              : roleFor(model.task) !== null && (
                  <Button size="sm" variant="secondary" onClick={onUse}>
                    {t("models.use")}
                  </Button>
                )}
            {/* The second job a speech model can hold: re-decoding a finished utterance more
              carefully than the live pass managed. The daemon has had the role since the
              pipeline did — `models.refine` — and nothing in the app could point at it, so the
              accurate-but-slow models in the catalogue had no use anybody could reach. */}
            {model.task === "asr" &&
              (asRefine ? (
                <span className="border-accent/40 text-accent text-micro rounded-full border px-2 py-0.5">
                  {t("models.in_use_refine")}
                </span>
              ) : (
                <Button size="sm" variant="ghost" onClick={onRefine}>
                  {t("models.use_refine")}
                </Button>
              ))}
            {/* No second "Đã cài" here: the state chip lives on the title line now, where the eye
              lands first. Two of them meant the same word twice on one card. */}
            <Button
              size="sm"
              variant={confirming ? "danger" : "ghost"}
              onClick={() => {
                if (confirming) onRemove();
                setConfirming(!confirming);
              }}
              onBlur={() => setConfirming(false)}
            >
              <Trash2 aria-hidden="true" className="me-1 size-3.5" />
              {confirming ? t("models.remove_confirm") : t("models.remove")}
            </Button>
          </div>
        ) : (
          <Button
            size="sm"
            variant="primary"
            busy={running}
            // A model the machine cannot hold is not offered. The download would finish and the
            // load would fail, which is the most expensive possible way to find out.
            disabled={!model.fits}
            onClick={onPull}
          >
            <HardDriveDownload aria-hidden="true" className="me-1 size-3.5" />
            {t("models.install")}
          </Button>
        )}
      </div>

      {/* The rest of what a model card owes a reader: the full per-language accuracy table,
          checksums, and the publisher's own README. All of it has existed in `summo_models::page`
          since the registry did — rendered for the CLI and never shown here. */}
      {/* A panel, not an accordion. Expanding a card in a two-column grid pushed its neighbour
          down the page and left the reader scrolling a card that had grown to four screens — and
          the thing being read is a document, which wants a column of its own. */}
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="text-fg-faint hover:text-fg text-micro mt-2 underline"
      >
        {t("models.details")}
      </button>
      <Sheet
        open={open}
        onOpenChange={setOpen}
        side="right"
        title={model.name}
        className="w-full max-w-xl"
      >
        <div className="h-full overflow-y-auto px-5 pb-8">
          <p className="text-fg-faint tabular text-micro">
            {model.id}
            {model.size_bytes > 0 && ` · ${size(model.size_bytes)}`}
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            {model.installed ? (
              <Button size="sm" variant="ghost" onClick={onRemove}>
                <Trash2 aria-hidden="true" className="me-1 size-3.5" />
                {t("models.remove")}
              </Button>
            ) : (
              <Button
                size="sm"
                variant="primary"
                busy={running}
                disabled={!model.fits}
                onClick={onPull}
              >
                <HardDriveDownload aria-hidden="true" className="me-1 size-3.5" />
                {t("models.install")}
              </Button>
            )}
            {model.installed && !inUse && roleFor(model.task) !== null && (
              <Button size="sm" variant="secondary" onClick={onUse}>
                {t("models.use")}
              </Button>
            )}
            {model.installed && inUse && optional && (
              <Button size="sm" variant="ghost" onClick={onStop}>
                {t("models.turn_off")}
              </Button>
            )}
          </div>
          {/* Every language anybody benchmarked, which is the comparison the card had to reduce
              to one number. Whisper's own table is the argument for and against it in four rows:
              95 % on English, 32 % on Vietnamese. */}
          {model.accuracy && model.accuracy.length > 0 && (
            <div className="border-line mt-4 border-t pt-4">
              <h4 className="text-meta font-medium">{t("models.measured_accuracy")}</h4>
              <dl className="mt-2 space-y-1.5">
                {model.accuracy.map((each) => (
                  <div key={each.lang} className="flex items-center gap-3">
                    <dt className="text-meta w-28 shrink-0 truncate">
                      {languageName(each.lang, locale)}
                    </dt>
                    <dd className="flex min-w-0 flex-1 items-center gap-2">
                      {/* A bar as well as a number: the gap between 95 % and 32 % is the whole
                          point and is far easier to see than to read. */}
                      <span className="bg-bg-soft h-1.5 min-w-0 flex-1 overflow-hidden rounded-full">
                        <span
                          className={cn(
                            "block h-full rounded-full",
                            each.accuracy >= 0.8 ? "bg-done" : "bg-blocked",
                          )}
                          style={{ width: `${Math.round(each.accuracy * 100)}%` }}
                        />
                      </span>
                      <span className="nums text-meta w-10 shrink-0 text-right tabular-nums">
                        {Math.round(each.accuracy * 100)}%
                      </span>
                    </dd>
                  </div>
                ))}
              </dl>
              <p className="text-fg-faint text-micro mt-2">{t("models.accuracy_note")}</p>
            </div>
          )}

          {/* Speed and latency, with the caveat attached rather than implied. */}
          {(model.speed || model.latency_ms) && (
            <div className="border-line mt-4 border-t pt-4">
              <h4 className="text-meta font-medium">{t("models.on_this_machine")}</h4>
              <dl className="text-meta mt-2 space-y-1">
                {model.speed && speedLabel(model.speed.rtf).times > 0 && (
                  <div className="flex justify-between gap-3">
                    <dt className="text-fg-dim">{t("models.vs_realtime")}</dt>
                    <dd className="nums tabular-nums">{speedLabel(model.speed.rtf).times}×</dd>
                  </div>
                )}
                {model.latency_ms ? (
                  <div className="flex justify-between gap-3">
                    <dt className="text-fg-dim">{t("models.latency")}</dt>
                    <dd className="nums tabular-nums">{model.latency_ms} ms</dd>
                  </div>
                ) : null}
                {model.rss_peak_mb ? (
                  <div className="flex justify-between gap-3">
                    <dt className="text-fg-dim">{t("models.while_running")}</dt>
                    <dd className="nums tabular-nums">{model.rss_peak_mb} MB</dd>
                  </div>
                ) : null}
                {model.accel && model.accel.length > 0 && (
                  <div className="flex justify-between gap-3">
                    <dt className="text-fg-dim">{t("models.accelerator")}</dt>
                    <dd>{model.accel.join(" · ")}</dd>
                  </div>
                )}
              </dl>
              {model.speed?.measured_here === false && (
                <p className="text-fg-faint text-micro mt-2">{t("models.measured_elsewhere")}</p>
              )}
            </div>
          )}

          <div className="border-line mt-4 border-t pt-4">
            {page === null ? (
              <p className="text-fg-faint text-micro">{t("common.loading")}</p>
            ) : page.markdown.trim() === "" ? (
              <p className="text-fg-faint text-micro">{t("models.no_details")}</p>
            ) : (
              <Markdown
                markdown={page.markdown.slice(Math.max(0, page.markdown.indexOf("\n## ")))}
                className="text-meta"
              />
            )}
          </div>
        </div>
      </Sheet>

      {/* The two facts that cost an afternoon if they turn up at the download instead of here. */}
      {!model.fits && (
        <p className="text-blocked text-micro mt-2">
          {t("models.needs_ram", { mb: model.min_ram_mb })}
        </p>
      )}
      {/* Whose model it is. Two rewrites to get here: first a licensing position written for a
          lawyer ("Summo không phân phối mô hình này. File tải thẳng từ nơi phát hành, theo giấy
          phép của họ."), then a sentence about where the bytes travel from, which is not something
          anyone choosing a model needs to think about. A credit is the whole of it. */}
      {model.attribution && !model.installed && (
        <p className="text-fg-faint text-micro mt-2">
          {t("models.upstream", { who: model.attribution || t("models.upstream_who") })}
        </p>
      )}

      {/* The bar here was honest about the percentage and silent about everything else, which on a
          611 MB model over a slow link is a bar that appears not to move. `Progress` adds the rate
          and the estimate, and is shared with the settings panel so the two agree. */}
      {running && job && <Progress install={job} className="mt-3" />}
      {job?.state === "failed" && (
        <p className="text-rec text-micro mt-2">{job.error ?? t("models.failed")}</p>
      )}
    </m.article>
  );
}
