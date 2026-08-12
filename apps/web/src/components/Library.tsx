import { motion } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  type LibraryView,
  type MeetingDetail,
  type MeetingSummary,
  type SearchHit,
} from "../lib/library";
import { ColourPicker, Dot, Finder } from "./library/Finder";
import { GENTLE, listItem, stagger } from "../lib/motion";

/** How long to wait after the last keystroke before searching. */
const SEARCH_DEBOUNCE_MS = 180;

// Keys, resolved at render: a module-level array is built before any provider exists, so baking
// the text in would freeze whichever language loaded first.
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
  // What is in the box, which leads what is in the URL by one debounce. A controlled input driven
  // straight from the route would push a history entry per keystroke and make the back button undo
  // the query one letter at a time.
  const [typed, setTyped] = useState(query);
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const today = useMemo(() => localDay(), []);

  const refresh = useCallback(async () => {
    try {
      // Joined here rather than in the client, because the daemon takes one query parameter: a
      // query string cannot express a repeated key that every deserialiser agrees on.
      setView(await client.view({ group, folder, tag: tags.join(","), colour }));
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, group, folder, tags, colour, say]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // A query arriving from outside — a reload, or the back button — is a new box, not an edit to
  // what is in it. Guarded so it does not fight the debounce below, which sets the same value one
  // beat after the user typed it.
  useEffect(() => {
    setTyped((current) => (current.trim() === query.trim() ? current : query));
  }, [query]);

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

  useEffect(() => {
    if (query.trim() === "") {
      setHits(null);
      return;
    }
    client
      .search(query)
      .then(setHits)
      .catch((e: unknown) => setError(say(e)));
  }, [client, query, say]);

  useEffect(() => {
    if (selected === null) {
      setDetail(null);
      return;
    }
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

        {hits === null && (
          <div
            className="bg-bg-soft flex gap-0.5 rounded-lg p-0.5"
            role="group"
            aria-label={t("library.group_by")}
          >
            {GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                className={cn(
                  "flex-1 rounded-md px-3 py-1.5 text-[13px] transition-colors",
                  group === g.value ? "bg-bg text-fg font-medium" : "text-fg-dim hover:text-fg",
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
              <motion.section
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
              </motion.section>
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
            className="border-blocked/30 bg-blocked-soft rounded-lg border px-2.5 py-1.5 text-[12px]"
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
          <p className="border-rec/30 bg-rec-soft text-rec rounded-lg border px-3 py-2 text-[13px]">
            {error}
          </p>
        )}

        {/* The way back to the list, which on a phone is the only way — there is no list beside
            this one to click away to. */}
        {narrow && selected !== null && (
          <button
            type="button"
            onClick={() => setSelected(null)}
            className="text-fg-dim hover:bg-bg-soft hover:text-fg -ms-1 mb-3 rounded-lg px-2 py-1 text-[13px]"
          >
            <span aria-hidden="true">←</span> {t("library.title")}
          </button>
        )}

        {detail ? (
          <MeetingPane
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
          <Dashboard stats={stats} onRecord={onRecord} />
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
    <motion.button
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
      <span className="tabular text-fg-faint shrink-0 text-[12px]">
        {meeting.kind === "note" ? "✎" : timeOfDay(meeting.date)}
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="flex min-w-0 items-center gap-1.5 leading-snug font-medium">
          {/* Beside the title rather than as a stripe down the row: a colour is one signal among
              several here, and a full-height bar reads as a status the way the selected row's does. */}
          {meeting.color && <Dot colour={meeting.color} />}
          <span className="truncate" data-testid="meeting-title">
            {meeting.title}
          </span>
        </span>
        <span className="text-fg-faint text-[12px]">
          {meeting.kind === "note" ? t("library.a_note") : formatDuration(meeting.duration, locale)}
          {meeting.participants.length > 0 && ` · ${meeting.participants.join(", ")}`}
          {meeting.kind !== "note" && !meeting.has_summary && t("meeting.not_summarised_suffix")}
        </span>
      </span>
    </motion.button>
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
              className="text-fg-dim [&_b]:text-fg my-0.5 ml-[42px] text-[13px] leading-normal [&_b]:font-medium"
            >
              {excerpt.t0 !== null && (
                <span className="tabular text-fg-faint mr-1.5 text-[11px]">
                  {timestamp(excerpt.t0)}
                </span>
              )}
              {excerpt.speaker && <b>{excerpt.speaker}</b>} {excerpt.text}
            </p>
          ))}
          {hit.matches > hit.excerpts.length && (
            <p className="text-fg-faint my-0.5 ml-[42px] text-[12px]">
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

function Dashboard({ stats, onRecord }: { stats?: Stats; onRecord: () => void }) {
  const { t, locale } = useI18n();
  if (!stats) return <p className="text-fg-faint mt-10 text-center">{t("library.loading")}</p>;
  return (
    <div className="max-w-2xl">
      <h2>{t("library.heading")}</h2>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(120px,1fr))] gap-2.5">
        <Tile label={t("library.meeting")} value={String(stats.meetings)} />
        <Tile
          label={t("library.recorded")}
          value={formatDuration(stats.total_duration, locale, "short")}
        />
        <Tile
          label={t("library.last_7")}
          value={`${stats.last_seven_days}`}
          note={formatDuration(stats.last_seven_days_duration, locale, "short")}
        />
        <Tile label={t("library.people")} value={String(stats.people)} />
        <Tile label={t("meeting.no_summary")} value={String(stats.without_summary)} />
      </div>
      <p className="text-fg-faint my-4 text-[13px]">{t("library.vault_hint")}</p>
      <button
        type="button"
        className="bg-accent text-accent-fg rounded-lg px-4 py-2 text-sm font-semibold transition-opacity hover:opacity-90"
        onClick={onRecord}
      >
        {t("library.record_new")}
      </button>
    </div>
  );
}

type Stats = NonNullable<LibraryView["stats"]>;

function Tile({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="rounded-card border-line bg-bg-soft flex flex-col gap-0.5 border p-3.5">
      <span
        className="tabular text-2xl font-semibold tracking-tight text-balance"
        data-testid="tile-value"
      >
        {value}
      </span>
      <span className="text-fg-dim text-[12px]">{label}</span>
      {note && <span className="text-fg-faint text-[11px]">{note}</span>}
    </div>
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

  // A different meeting is a different set of fields, not an edit to the current ones.
  useEffect(() => {
    setTitle(summary.title);
    setTags(summary.tags.join(", "));
    setConfirming(false);
  }, [summary.id, summary.title, summary.tags]);

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
        <p className="text-fg-faint text-[13px]">
          {dayLabel(summary.day, localDay(), words)} · {timeOfDay(summary.date)} ·{" "}
          {formatDuration(summary.duration, locale)}
          {detail.audio.length > 0 &&
            ` · ${t("library.recordings", { count: detail.audio.length })}`}
        </p>

        <div className="border-line mt-2 flex flex-wrap items-center gap-x-4 gap-y-2.5 border-b pb-4">
          <label className="text-fg-faint flex items-center gap-1.5 text-[13px]">
            {t("library.by_folder")}
            <select
              className="border-line bg-bg-soft text-fg focus-visible:border-accent rounded-lg border px-2 py-1 text-[13px] focus:outline-none"
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
          <label className="text-fg-faint flex min-w-0 grow basis-56 items-center gap-1.5 text-[13px]">
            {t("library.by_tag")}
            <input
              className="border-line bg-bg-soft text-fg focus-visible:border-accent min-w-32 flex-1 rounded-lg border px-2 py-1 text-[13px] focus:outline-none"
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
            <span className="text-fg-dim flex items-center gap-1.5 text-[13px]">
              {t("library.trash_confirm")}
              <button
                type="button"
                onClick={onTrash}
                disabled={busy}
                className="border-rec text-rec hover:bg-rec-soft rounded-md border px-2.5 py-1 text-[13px] disabled:opacity-50"
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
              <span className="tabular text-fg-faint mr-1.5 text-[11px]">
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
