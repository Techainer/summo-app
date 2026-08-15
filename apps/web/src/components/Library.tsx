import type { LucideIcon } from "lucide-react";
import {
  AudioLines,
  CalendarRange,
  CircleAlert,
  Clock,
  Mic,
  PencilLine,
  Users,
} from "lucide-react";
import { m } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Recent } from "./library/Recent";
import { Avatar, Button, SectionTitle, Wave } from "./ui";
import { cn } from "../lib/cn";
import { formatDuration } from "../lib/duration";
import { useI18n, useT } from "../i18n/context";
import { useIsNarrow } from "../lib/breakpoint";
import { useErrorText } from "../lib/errors";
import {
  LibraryClient,
  dayLabel,
  groupLabel,
  localDay,
  timeOfDay,
  timestamp,
  type GroupBy,
  type Kind,
  type LibraryView,
  type MeetingDetail,
  type MeetingSummary,
  type SearchHit,
} from "../lib/library";
import { ColourPicker, Dot, Finder } from "./library/Finder";
import { GENTLE, listItem, stagger } from "../lib/motion";
import { useRefresh } from "../lib/use-load";

/** How long to wait after the last keystroke before searching. */
const SEARCH_DEBOUNCE_MS = 180;

// Keys, resolved at render: a module-level array is built before any provider exists, so baking
// the text in would freeze whichever language loaded first.
// Keys, resolved at render, for the same reason as `GROUPS` below.
const KINDS: { value: Kind | undefined; labelKey: string }[] = [
  { value: undefined, labelKey: "library.kind_all" },
  { value: "meeting", labelKey: "library.kind_meeting" },
  { value: "note", labelKey: "library.kind_note" },
];

const GROUPS: { value: GroupBy; labelKey: string }[] = [
  { value: "day", labelKey: "library.by_day" },
  { value: "week", labelKey: "library.by_week" },
  { value: "folder", labelKey: "library.by_folder" },
];

interface Props {
  client: LibraryClient;
  /**
   * Which folder is being browsed, `""` for the root and `undefined` for all of them.
   *
   * Every filter below is owned by the route rather than by this component. The folder has to be,
   * because the app's own sidebar sets it too and a filter with two owners shows whichever was
   * told last. The rest follow so there is one answer to "what is on screen" instead of two
   * mechanisms — and so a narrowed view survives a reload and steps back out with the back button.
   */
  folder: string | undefined;
  tags: string[];
  colour: string | undefined;
  query: string;
  onFolder: (folder: string | undefined) => void;
  onTags: (tags: string[]) => void;
  onColour: (colour: string | undefined) => void;
  onQuery: (query: string) => void;
  /** Called when the user wants to record instead of read. */
  onRecord: () => void;
  /** Open a meeting on its own screen, where the player and transcript live. */
  onOpen?: (id: string) => void;
}

/**
 * The library: every meeting already on disk.
 *
 * State is deliberately shallow — a filter change refetches rather than deriving a new view from a
 * cached one. The daemon rescans in a few milliseconds, and a derived view is how a list ends up
 * disagreeing with the files it claims to describe.
 */
/**
 * The words a date heading needs, in the interface's own language.
 *
 * A hook rather than a prop threaded down: both the list and the detail pane render dates, and the
 * alternative was passing the same five strings through two component boundaries.
 */
function useDayWords() {
  const { t, locale } = useI18n();
  return useMemo(
    () => ({
      locale,
      today: t("library.today"),
      yesterday: t("library.yesterday"),
      week: t("library.week"),
      unfiled: t("library.unfiled_group"),
    }),
    [locale, t],
  );
}

export function Library({
  client,
  folder,
  tags,
  colour,
  query,
  onFolder,
  onTags,
  onColour,
  onQuery,
  onRecord,
  onOpen,
}: Props) {
  const t = useT();
  const words = useDayWords();
  const narrow = useIsNarrow();
  const say = useErrorText();
  const [view, setView] = useState<LibraryView | null>(null);
  const [group, setGroup] = useState<GroupBy>("day");
  /**
   * Which kind of thing is being looked at, or all of them.
   *
   * Local rather than in the route, unlike the folder and tag filters: those narrow *what exists*
   * and are worth a shareable URL, while this is a lens on the same shelf and switching it is a
   * glance rather than a destination.
   */
  const [kind, setKind] = useState<Kind | undefined>(undefined);
  // What is in the box, which leads what is in the URL by one debounce. A controlled input driven
  // straight from the route would push a history entry per keystroke and make the back button undo
  // the query one letter at a time.
  const [typed, setTyped] = useState(query);
  const [fetchedHits, setHits] = useState<SearchHit[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [fetchedDetail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const today = useMemo(() => localDay(), []);

  // Derived, not stored. With no query there are no hits and with nothing selected there is no
  // detail — both are facts about the query and the selection, and keeping a second copy of them in
  // state meant every keystroke that emptied the box wrote `null` over a `null`.
  const hits = query.trim() === "" ? null : fetchedHits;
  const detail = selected === null ? null : fetchedDetail;

  const refresh = useCallback(async () => {
    try {
      // Joined here rather than in the client, because the daemon takes one query parameter: a
      // query string cannot express a repeated key that every deserialiser agrees on.
      setView(await client.view({ group, kind, folder, tag: tags.join(","), colour }));
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, group, kind, folder, tags, colour, say]);

  useRefresh(refresh);

  // A query arriving from outside — a reload, or the back button — is a new box, not an edit to
  // what is in it.
  //
  // Adjusted during render rather than in an effect. React documents this exact shape for "reset
  // state when a prop changes": compare against the value the state was derived from, and set both
  // on the spot. The effect version rendered once with the stale text and once with the new, which
  // is a visible flicker in the search box on every back-button press.
  const [seenQuery, setSeenQuery] = useState(query);
  if (seenQuery !== query) {
    setSeenQuery(query);
    if (typed.trim() !== query.trim()) setTyped(query);
  }

  // Searching on every keystroke would send a request per character; waiting for a pause sends one
  // per word, which is what a person means by typing anyway. The same pause is when the URL
  // catches up, so history gets one entry per search rather than one per letter.
  const searchTimer = useRef<number | null>(null);
  useEffect(() => {
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    if (typed.trim() === query.trim()) return undefined;
    searchTimer.current = window.setTimeout(() => onQuery(typed.trim()), SEARCH_DEBOUNCE_MS);
    return () => {
      if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    };
  }, [typed, query, onQuery]);

  // No `setHits(null)` for the empty query: "there is no search" is not a state to store, it is a
  // fact about the query, and `shown` below reads it off the query directly. Storing it meant a
  // synchronous state write on every keystroke that emptied the box.
  useEffect(() => {
    if (query.trim() === "") return;
    client
      .search(query)
      .then(setHits)
      .catch((e: unknown) => setError(say(e)));
  }, [client, query, say]);

  useEffect(() => {
    if (selected === null) return;
    client
      .detail(selected)
      .then(setDetail)
      .catch((e: unknown) => setError(say(e)));
  }, [client, selected, say]);

  const mutate = useCallback(
    async (action: () => Promise<unknown>) => {
      setBusy(true);
      try {
        await action();
        await refresh();
        if (selected !== null) setDetail(await client.detail(selected).catch(() => null));
        setError(null);
      } catch (e) {
        setError(say(e));
      } finally {
        setBusy(false);
      }
    },
    [client, refresh, selected, say],
  );

  const stats = view?.stats;

  return (
    <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[320px_1fr]" data-testid="library">
      {/* On a phone the two panes take turns; on a desktop they sit side by side.
       *
       * They used to stack, with the list capped at 45% of the height. At 390 px that is 167 px of
       * window for 754 px of content — the filters filled it and the meetings were below the fold,
       * so the library looked empty on the device most likely to open it. Taking turns is what a
       * phone does with a master and a detail, and it is why `selected` gets a way back out. */}
      <aside
        hidden={narrow && selected !== null}
        className="border-line flex min-h-0 flex-col gap-2.5 overflow-y-auto border-b p-3 md:border-r md:border-b-0"
      >
        <div className="flex gap-2">
          <input
            className="border-line bg-bg-soft text-fg focus-visible:border-accent w-full rounded-lg border px-3 py-2 text-sm focus:outline-none"
            type="search"
            value={typed}
            placeholder={t("library.search_placeholder")}
            aria-label={t("library.search")}
            onChange={(e) => setTyped(e.target.value)}
          />
        </div>

        {/* What kind of thing, before how it is grouped.
            
            The vault has always held recordings and typed notes side by side and there was no way
            to ask for one — which is how a workspace ends up telling the user that meetings are the
            product. `Tất cả` first and selected, because both together is the honest default. */}
        {hits === null && (
          <div
            className="bg-bg-soft flex gap-0.5 rounded-[var(--radius-pill)] p-0.5"
            role="group"
            aria-label={t("library.filter_kind")}
          >
            {KINDS.map((k) => (
              <button
                key={k.labelKey}
                type="button"
                aria-pressed={kind === k.value}
                className={cn(
                  "text-meta flex-1 rounded-[var(--radius-pill)] px-3 py-1.5 transition-colors",
                  kind === k.value
                    ? "bg-bg-raised text-fg font-medium shadow-[var(--shadow-sm)]"
                    : "text-fg-dim hover:text-fg",
                )}
                onClick={() => setKind(k.value)}
              >
                {t(k.labelKey)}
              </button>
            ))}
          </div>
        )}

        {hits === null && (
          <div
            className="bg-bg-soft flex gap-0.5 rounded-[var(--radius-pill)] p-0.5"
            role="group"
            aria-label={t("library.group_by")}
          >
            {GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                className={cn(
                  "text-meta flex-1 rounded-[var(--radius-pill)] px-3 py-1.5 transition-colors",
                  group === g.value
                    ? "bg-bg-raised text-fg font-medium shadow-[var(--shadow-sm)]"
                    : "text-fg-dim hover:text-fg",
                )}
                onClick={() => setGroup(g.value)}
              >
                {t(g.labelKey)}
              </button>
            ))}
          </div>
        )}

        {/* Hidden while searching: a full-text query is its own way of narrowing, and leaving the
            filters on screen invites the reading that they are still applied to the hits. */}
        {view && hits === null && (
          <Finder
            view={view}
            folder={folder}
            tags={tags}
            colour={colour}
            onFolder={onFolder}
            onTags={onTags}
            onColour={onColour}
          />
        )}

        <div className="min-h-0 flex-1" data-testid="meeting-list">
          {hits !== null ? (
            <SearchResults hits={hits} onSelect={setSelected} selected={selected} />
          ) : (
            (view?.groups ?? []).map((g) => (
              // Keyed on the filter as well as the day, so changing a filter re-runs the stagger:
              // the list assembling is what tells the user the change took effect on a screen
              // where the only other feedback is that some rows are gone.
              <m.section
                key={`${g.key}-${folder ?? ""}-${tags.join()}-${colour ?? ""}`}
                className="mb-3.5"
                initial="hidden"
                animate="shown"
                transition={stagger(g.meetings.length)}
              >
                <h3 data-testid="group-heading">{groupLabel(g.key, group, today, words)}</h3>
                {g.meetings.map((m) => (
                  <MeetingRow
                    key={m.id}
                    meeting={m}
                    selected={m.id === selected}
                    onSelect={() => setSelected(m.id)}
                    onOpen={onOpen ? () => onOpen(m.id) : undefined}
                  />
                ))}
              </m.section>
            ))
          )}

          {hits === null && view?.total === 0 && (
            <p className="text-fg-faint mt-10 text-center">
              {t("library.empty")}
              <button type="button" className="text-accent hover:underline" onClick={onRecord}>
                {t("library.empty_cta")}
              </button>
            </p>
          )}
          {hits?.length === 0 && (
            <p className="text-fg-faint mt-10 text-center">{t("library.no_hits", { query })}</p>
          )}
        </div>

        {/* A file that would not parse is on disk but not on screen; saying so is the only way the
            user can go and fix it. */}
        {view?.skipped.map((s) => (
          <p
            key={s.path}
            className="border-blocked/30 bg-blocked-soft text-micro rounded-lg border px-2.5 py-1.5"
          >
            {t("library.unreadable", { path: s.path, reason: s.reason })}
          </p>
        ))}
      </aside>

      <section
        hidden={narrow && selected === null}
        className="min-h-0 overflow-y-auto px-4 py-5 md:px-7 md:py-6"
      >
        {error && (
          <p className="border-rec/30 bg-rec-soft text-rec text-meta rounded-lg border px-3 py-2">
            {error}
          </p>
        )}

        {/* The way back to the list, which on a phone is the only way — there is no list beside
            this one to click away to. */}
        {narrow && selected !== null && (
          <button
            type="button"
            onClick={() => setSelected(null)}
            className="text-fg-dim hover:bg-bg-soft hover:text-fg text-meta -ms-1 mb-3 rounded-lg px-2 py-1"
          >
            <span aria-hidden="true">←</span> {t("library.title")}
          </button>
        )}

        {detail ? (
          <MeetingPane
            key={detail.summary.id}
            detail={detail}
            folders={view?.folders ?? []}
            palette={view?.palette ?? []}
            busy={busy}
            onRename={(title) => void mutate(() => client.rename(detail.summary.id, title))}
            onMove={(to) => void mutate(() => client.moveTo(detail.summary.id, to))}
            onTags={(tags) => void mutate(() => client.setTags(detail.summary.id, tags))}
            onColour={(colour) => void mutate(() => client.setColour(detail.summary.id, colour))}
            onTrash={() =>
              void mutate(async () => {
                await client.trash(detail.summary.id);
                setSelected(null);
              })
            }
          />
        ) : (
          <Dashboard stats={stats} onRecord={onRecord} onOpen={setSelected} />
        )}
      </section>
    </div>
  );
}

function MeetingRow({
  meeting,
  selected,
  onSelect,
  onOpen,
}: {
  meeting: MeetingSummary;
  selected: boolean;
  onSelect: () => void;
  onOpen?: () => void;
}) {
  const { t, locale } = useI18n();
  return (
    <m.button
      variants={listItem}
      transition={GENTLE}
      type="button"
      className={cn(
        "flex w-full items-baseline gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors",
        selected ? "bg-bg-soft shadow-[inset_2px_0_0_var(--color-accent)]" : "hover:bg-bg-soft",
      )}
      data-testid="meeting-row"
      onClick={onSelect}
      // A single click previews beside the list; a double click commits to the full screen, the
      // way a file manager works. Enter does the same for the keyboard.
      onDoubleClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" && onOpen) {
          e.preventDefault();
          onOpen();
        }
      }}
      aria-current={selected}
    >
      {/* A note has no time of day worth showing — it was typed, not scheduled — so the column
          carries a mark instead. Same width either way, so the titles still line up. */}
      <span className="tabular text-fg-faint text-micro shrink-0">
        {meeting.kind === "note" ? (
          <PencilLine aria-hidden="true" className="mx-auto size-3.5 stroke-[1.75]" />
        ) : (
          timeOfDay(meeting.date)
        )}
      </span>

      {/* A silhouette per recording. Two rows that say "42 phút · Bạn, Ngọc" are the same row until
          you read them; two waveforms are different at a glance, which is what makes a page of
          recordings a page rather than a paragraph. Notes have none — they were typed. */}
      {meeting.kind !== "note" && (
        <span className="text-accent/60 hidden h-6 w-10 shrink-0 sm:block">
          <Wave seed={meeting.id} bars={11} />
        </span>
      )}
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5 leading-snug font-medium">
          {/* Beside the title rather than as a stripe down the row: a colour is one signal among
              several here, and a full-height bar reads as a status the way the selected row's does. */}
          {meeting.color && <Dot colour={meeting.color} />}
          <span className="truncate" data-testid="meeting-title">
            {meeting.title}
          </span>
        </span>
        <span className="text-fg-faint text-micro flex items-center gap-1.5">
          {/* The discs carry who, the text carries how long. Names stay in the line beside them:
              four initials are a way to recognise a row already read, not a way to read it. */}
          {meeting.participants.length > 0 && (
            <span className="flex shrink-0 -space-x-1">
              {meeting.participants.slice(0, 3).map((who) => (
                <Avatar key={who} name={who} size="sm" className="ring-bg ring-2" />
              ))}
            </span>
          )}
          <span className="truncate">
            {meeting.kind === "note"
              ? t("library.a_note")
              : formatDuration(meeting.duration, locale)}
            {meeting.participants.length > 0 && ` · ${meeting.participants.join(", ")}`}
            {meeting.kind !== "note" && !meeting.has_summary && t("meeting.not_summarised_suffix")}
          </span>
        </span>
      </span>
    </m.button>
  );
}

function SearchResults({
  hits,
  selected,
  onSelect,
}: {
  hits: SearchHit[];
  selected: string | null;
  onSelect: (id: string) => void;
}) {
  const t = useT();
  return (
    <>
      {hits.map((hit) => (
        <section key={hit.meeting.id} className="mb-3.5">
          <MeetingRow
            meeting={hit.meeting}
            selected={hit.meeting.id === selected}
            onSelect={() => onSelect(hit.meeting.id)}
          />
          {hit.excerpts.map((excerpt, i) => (
            <p
              key={i}
              data-testid="excerpt"
              className="text-fg-dim [&_b]:text-fg text-meta my-0.5 ml-[42px] leading-normal [&_b]:font-medium"
            >
              {excerpt.t0 !== null && (
                <span className="tabular text-fg-faint text-micro mr-1.5">
                  {timestamp(excerpt.t0)}
                </span>
              )}
              {excerpt.speaker && <b>{excerpt.speaker}</b>} {excerpt.text}
            </p>
          ))}
          {hit.matches > hit.excerpts.length && (
            <p className="text-fg-faint text-micro my-0.5 ml-[42px]">
              {t("library.more_lines", {
                count: hit.matches - hit.excerpts.length,
              })}
            </p>
          )}
        </section>
      ))}
    </>
  );
}

/**
 * The right-hand pane with nothing selected.
 *
 * It was five outlined boxes, a sentence and a button, over four hundred pixels of background — the
 * screen that most deserved the complaint that started this redesign. What a vault screen owes the
 * person opening it is a picture of what is *in* the vault, so: the numbers, then the recordings
 * themselves, each with the silhouette and the faces that make a list of them scannable.
 */
function Dashboard({
  stats,
  onRecord,
  onOpen,
}: {
  stats?: Stats;
  onRecord: () => void;
  onOpen: (id: string) => void;
}) {
  const { t, locale } = useI18n();
  if (!stats) return <p className="text-fg-faint mt-10 text-center">{t("library.loading")}</p>;

  return (
    <div className="relative mx-auto max-w-4xl">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -inset-x-6 -top-8 h-56 bg-[image:var(--gradient-page)]"
      />

      <div className="relative">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <h2 className="text-display font-semibold">{t("library.heading")}</h2>
          <Button variant="primary" onClick={onRecord}>
            <Mic aria-hidden="true" className="me-1.5 size-4" />
            {t("library.record_new")}
          </Button>
        </div>

        <m.div
          initial="hidden"
          animate="shown"
          transition={stagger(4)}
          className="mt-5 grid grid-cols-2 gap-3 lg:grid-cols-4"
        >
          <Tile
            icon={AudioLines}
            label={t("library.meeting")}
            value={String(stats.meetings)}
            tone="accent"
          />
          <Tile
            icon={Clock}
            label={t("library.recorded")}
            value={formatDuration(stats.total_duration, locale, "short")}
          />
          <Tile
            icon={CalendarRange}
            label={t("library.last_7")}
            value={`${stats.last_seven_days}`}
            note={formatDuration(stats.last_seven_days_duration, locale, "short")}
          />
          <Tile icon={Users} label={t("library.people")} value={String(stats.people)} />
        </m.div>

        {/* Unsummarised is not a statistic, it is a chore: it belongs beside the others only when
            there are some, and coloured like the work it is. */}
        {stats.without_summary > 0 && (
          <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta mt-3 inline-flex items-center gap-2 rounded-[var(--radius-pill)] border px-3 py-1.5">
            <CircleAlert aria-hidden="true" className="size-3.5" />
            {t("meeting.no_summary")} · {stats.without_summary}
          </p>
        )}

        <section className="mt-7 flex flex-col gap-2.5">
          <SectionTitle>{t("library.recent_heading")}</SectionTitle>
          <Recent limit={6} columns={2} onOpen={(entry) => onOpen(entry.id)} />
        </section>

        <p className="text-fg-faint text-meta mt-7">{t("library.vault_hint")}</p>
      </div>
    </div>
  );
}

type Stats = NonNullable<LibraryView["stats"]>;

function Tile({
  label,
  value,
  note,
  icon: Icon,
  tone,
}: {
  label: string;
  value: string;
  note?: string;
  icon: LucideIcon;
  tone?: "accent";
}) {
  return (
    <m.div
      variants={listItem}
      className="rounded-card border-line bg-bg-soft flex flex-col gap-0.5 border p-3.5 shadow-[var(--shadow-sm)]"
    >
      <span
        className={cn(
          "mb-1.5 grid size-8 place-items-center rounded-[var(--radius-card)]",
          tone === "accent" ? "bg-accent-soft text-accent" : "bg-bg-raised text-fg-faint",
        )}
      >
        <Icon aria-hidden="true" className="size-4 stroke-[1.75]" />
      </span>
      {/* `break-keep`: Japanese and Chinese have no spaces, so a browser is free to break a line
          anywhere at all — and `text-balance` then took it up on that, splitting 時間 down the
          middle and justifying the halves across two lines. It read as a rendering fault rather
          than as a number. `word-break: keep-all` keeps a word whole; the wrap still happens, at
          the space between the hours and the minutes, where a reader expects it. */}
      <span
        className="nums text-2xl font-semibold tracking-tight text-balance break-keep"
        data-testid="tile-value"
      >
        {value}
      </span>
      <span className="text-fg-dim text-micro">{label}</span>
      {note && <span className="text-fg-faint text-micro">{note}</span>}
    </m.div>
  );
}

function MeetingPane({
  detail,
  folders,
  palette,
  busy,
  onRename,
  onMove,
  onTags,
  onColour,
  onTrash,
}: {
  detail: MeetingDetail;
  folders: string[];
  palette: string[];
  busy: boolean;
  onRename: (title: string) => void;
  onMove: (folder: string) => void;
  onTags: (tags: string[]) => void;
  onColour: (colour: string | null) => void;
  onTrash: () => void;
}) {
  const { t, locale } = useI18n();
  const { summary } = detail;
  const [title, setTitle] = useState(summary.title);
  const [tags, setTags] = useState(summary.tags.join(", "));
  const [confirming, setConfirming] = useState(false);

  // A different meeting is a different set of fields, not an edit to the current ones — and the
  // parent keys this component on the meeting id, so a different meeting is a different component
  // and the fields above start from its own values. The effect that used to copy them in ran after
  // the first paint, so switching meetings showed the previous title for one frame.

  const words = useDayWords();
  const known = useMemo(
    () => [...new Set([...folders, summary.folder])].sort(),
    [folders, summary.folder],
  );

  return (
    <article className="max-w-3xl" data-testid="meeting">
      {/* Stacked, not a row. These are a title, a caption and a set of controls — three different
          kinds of thing — and laying them side by side clipped the title behind the date and ran
          the folder, tags and colour off the edge of the pane. */}
      <header className="flex flex-col gap-1">
        <input
          className="text-fg hover:border-line focus:border-accent w-full border-0 border-b border-transparent bg-transparent px-0 py-0.5 text-[22px] font-semibold tracking-tight focus:outline-none"
          value={title}
          aria-label={t("meeting.title_label")}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title !== summary.title && onRename(title.trim())}
        />
        <p className="text-fg-faint text-meta">
          {dayLabel(summary.day, localDay(), words)} · {timeOfDay(summary.date)} ·{" "}
          {formatDuration(summary.duration, locale)}
          {detail.audio.length > 0 &&
            ` · ${t("library.recordings", { count: detail.audio.length })}`}
        </p>

        <div className="border-line mt-2 flex flex-wrap items-center gap-x-4 gap-y-2.5 border-b pb-4">
          <label className="text-fg-faint text-meta flex items-center gap-1.5">
            {t("library.by_folder")}
            <select
              className="border-line bg-bg-soft text-fg focus-visible:border-accent text-meta rounded-lg border px-2 py-1 focus:outline-none"
              value={summary.folder}
              disabled={busy}
              onChange={(e) => onMove(e.target.value)}
              aria-label={t("library.by_folder")}
            >
              {known.map((f) => (
                <option key={f} value={f}>
                  {f === "" ? t("library.unfiled") : f}
                </option>
              ))}
            </select>
          </label>
          <label className="text-fg-faint text-meta flex min-w-0 grow basis-56 items-center gap-1.5">
            {t("library.by_tag")}
            <input
              className="border-line bg-bg-soft text-fg focus-visible:border-accent text-meta min-w-32 flex-1 rounded-lg border px-2 py-1 focus:outline-none"
              value={tags}
              disabled={busy}
              aria-label={t("library.by_tag")}
              placeholder="weekly, product"
              onChange={(e) => setTags(e.target.value)}
              onBlur={() =>
                onTags(
                  tags
                    .split(",")
                    .map((t) => t.trim())
                    .filter(Boolean),
                )
              }
            />
          </label>
          <ColourPicker
            palette={palette}
            chosen={summary.color}
            disabled={busy}
            onChoose={onColour}
          />
          {confirming ? (
            <span className="text-fg-dim text-meta flex items-center gap-1.5">
              {t("library.trash_confirm")}
              <button
                type="button"
                onClick={onTrash}
                disabled={busy}
                className="border-rec text-rec hover:bg-rec-soft text-meta rounded-md border px-2.5 py-1 disabled:opacity-50"
              >
                {t("library.trash_yes")}
              </button>
              <button type="button" onClick={() => setConfirming(false)}>
                {t("common.cancel")}
              </button>
            </span>
          ) : (
            <button type="button" className="ghost" onClick={() => setConfirming(true)}>
              {t("common.delete")}
            </button>
          )}
        </div>
      </header>

      {detail.sections.map((s) => (
        <section key={s.heading} className="mt-5">
          <h3>{s.heading}</h3>
          <p>{s.body}</p>
        </section>
      ))}

      <section className="mt-5">
        <h3>{t("library.transcript_lines", { count: detail.transcript.length })}</h3>
        {detail.transcript.length === 0 && (
          <p className="text-fg-faint mt-10 text-center">{t("library.no_content")}</p>
        )}
        <ol className="m-0 list-none p-0">
          {detail.transcript.map((segment) => (
            <li key={segment.seq} data-testid="transcript-line">
              <span className="tabular text-fg-faint text-micro mr-1.5">
                {timestamp(segment.t0)}
              </span>
              <b>{segment.speaker ?? "?"}</b>
              <span>{segment.text}</span>
            </li>
          ))}
        </ol>
      </section>
    </article>
  );
}
