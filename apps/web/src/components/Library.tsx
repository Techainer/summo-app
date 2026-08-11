import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../lib/cn";
import { useI18n, useT } from "../i18n/context";
import { useErrorText } from "../lib/errors";
import {
  LibraryClient,
  dayLabel,
  formatDuration,
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

export function Library({ client, onRecord, onOpen }: Props) {
  const t = useT();
  const words = useDayWords();
  const say = useErrorText();
  const [view, setView] = useState<LibraryView | null>(null);
  const [group, setGroup] = useState<GroupBy>("day");
  const [folder, setFolder] = useState<string | undefined>();
  const [tag, setTag] = useState<string | undefined>();
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const today = useMemo(() => localDay(), []);

  const refresh = useCallback(async () => {
    try {
      setView(await client.view({ group, folder, tag }));
      setError(null);
    } catch (e) {
      setError(say(e));
    }
  }, [client, group, folder, tag]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Searching on every keystroke would send a request per character; waiting for a pause sends one
  // per word, which is what a person means by typing anyway.
  const searchTimer = useRef<number | null>(null);
  useEffect(() => {
    if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    if (query.trim() === "") {
      setHits(null);
      return undefined;
    }
    searchTimer.current = window.setTimeout(() => {
      client
        .search(query)
        .then(setHits)
        .catch((e: unknown) => setError(say(e)));
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      if (searchTimer.current !== null) window.clearTimeout(searchTimer.current);
    };
  }, [client, query]);

  useEffect(() => {
    if (selected === null) {
      setDetail(null);
      return;
    }
    client
      .detail(selected)
      .then(setDetail)
      .catch((e: unknown) => setError(say(e)));
  }, [client, selected]);

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
    [client, refresh, selected],
  );

  const stats = view?.stats;

  return (
    <div className="grid h-full min-h-0 grid-cols-1 md:grid-cols-[320px_1fr]" data-testid="library">
      <aside className="flex min-h-0 max-h-[45%] flex-col gap-2.5 overflow-y-auto border-b border-line p-3 md:max-h-none md:border-r md:border-b-0">
        <div className="flex gap-2">
          <input
            className="w-full rounded-lg border border-line bg-bg-soft px-3 py-2 text-sm text-fg focus:outline-none focus-visible:border-accent"
            type="search"
            value={query}
            placeholder={t("library.search_placeholder")}
            aria-label={t("library.search")}
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        {hits === null && (
          <div className="flex gap-0.5 rounded-lg bg-bg-soft p-0.5" role="group" aria-label={t("library.group_by")}>
            {GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                className={cn(
                  "flex-1 rounded-md px-3 py-1.5 text-[13px] transition-colors",
                  group === g.value
                    ? "bg-bg font-medium text-fg"
                    : "text-fg-dim hover:text-fg",
                )}
                onClick={() => setGroup(g.value)}
              >
                {t(g.labelKey)}
              </button>
            ))}
          </div>
        )}

        {(folder !== undefined || tag !== undefined) && (
          <div className="flex flex-wrap gap-1.5">
            {folder !== undefined && (
              <button type="button" className="rounded-full border border-fg-faint bg-bg-soft px-2.5 py-0.5 text-[12px] text-fg" onClick={() => setFolder(undefined)}>
                📁 {folder === "" ? t("library.unfiled") : folder} ✕
              </button>
            )}
            {tag !== undefined && (
              <button type="button" className="rounded-full border border-fg-faint bg-bg-soft px-2.5 py-0.5 text-[12px] text-fg" onClick={() => setTag(undefined)}>
                #{tag} ✕
              </button>
            )}
          </div>
        )}

        <div className="min-h-0 flex-1" data-testid="meeting-list">
          {hits !== null ? (
            <SearchResults hits={hits} onSelect={setSelected} selected={selected} />
          ) : (
            (view?.groups ?? []).map((g) => (
              <section key={g.key} className="mb-3.5">
                <h3>{groupLabel(g.key, group, today, words)}</h3>
                {g.meetings.map((m) => (
                  <MeetingRow
                    key={m.id}
                    meeting={m}
                    selected={m.id === selected}
                    onSelect={() => setSelected(m.id)}
                    onOpen={onOpen ? () => onOpen(m.id) : undefined}
                  />
                ))}
              </section>
            ))
          )}

          {hits === null && view?.total === 0 && (
            <p className="mt-10 text-center text-fg-faint">
              {t("library.empty")}
              <button type="button" className="text-accent hover:underline" onClick={onRecord}>
                {t("library.empty_cta")}
              </button>
            </p>
          )}
          {hits?.length === 0 && <p className="mt-10 text-center text-fg-faint">{t("library.no_hits", { query })}</p>}
        </div>

        {view && view.folders.length > 1 && hits === null && (
          <nav className="flex flex-wrap gap-1.5 border-t border-line pt-2" aria-label={t("library.filter_folder")}>
            {view.folders.map((f) => (
              <button key={f} type="button" onClick={() => setFolder(f)}>
                {f === "" ? t("library.unfiled") : f}
              </button>
            ))}
          </nav>
        )}

        {view && view.tags.length > 0 && hits === null && (
          <nav className="flex flex-wrap gap-1.5 border-t border-line pt-2" aria-label={t("library.filter_tag")}>
            {view.tags.map((t) => (
              <button key={t.name} type="button" onClick={() => setTag(t.name)}>
                #{t.name} <span>{t.count}</span>
              </button>
            ))}
          </nav>
        )}

        {/* A file that would not parse is on disk but not on screen; saying so is the only way the
            user can go and fix it. */}
        {view?.skipped.map((s) => (
          <p
            key={s.path}
            className="rounded-lg border border-blocked/30 bg-blocked-soft px-2.5 py-1.5 text-[12px]"
          >
            {t("library.unreadable", { path: s.path, reason: s.reason })}
          </p>
        ))}
      </aside>

      <section className="min-h-0 overflow-y-auto px-7 py-6">
        {error && (
          <p className="rounded-lg border border-rec/30 bg-rec-soft px-3 py-2 text-[13px] text-rec">
            {error}
          </p>
        )}

        {detail ? (
          <MeetingPane
            detail={detail}
            folders={view?.folders ?? []}
            busy={busy}
            onRename={(title) => void mutate(() => client.rename(detail.summary.id, title))}
            onMove={(to) => void mutate(() => client.moveTo(detail.summary.id, to))}
            onTags={(tags) => void mutate(() => client.setTags(detail.summary.id, tags))}
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
  const t = useT();
  return (
    <button
      type="button"
      className={cn(
        "flex w-full items-baseline gap-2.5 rounded-lg px-2 py-1.5 text-left transition-colors",
        selected ? "bg-bg-soft shadow-[inset_2px_0_0_var(--color-accent)]" : "hover:bg-bg-soft",
      )}
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
      <span className="tabular shrink-0 text-[12px] text-fg-faint">
        {meeting.kind === "note" ? "✎" : timeOfDay(meeting.date)}
      </span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="leading-snug font-medium">{meeting.title}</span>
        <span className="text-[12px] text-fg-faint">
          {meeting.kind === "note"
            ? t("library.a_note")
            : formatDuration(meeting.duration)}
          {meeting.participants.length > 0 && ` · ${meeting.participants.join(", ")}`}
          {meeting.kind !== "note" && !meeting.has_summary && t("meeting.not_summarised_suffix")}
        </span>
      </span>
    </button>
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
            <p key={i} className="my-0.5 ml-[42px] text-[13px] leading-normal text-fg-dim [&_b]:font-medium [&_b]:text-fg">
              {excerpt.t0 !== null && <span className="tabular mr-1.5 text-[11px] text-fg-faint">{timestamp(excerpt.t0)}</span>}
              {excerpt.speaker && <b>{excerpt.speaker}</b>} {excerpt.text}
            </p>
          ))}
          {hit.matches > hit.excerpts.length && (
            <p className="my-0.5 ml-[42px] text-[12px] text-fg-faint">{t("library.more_lines", { count: hit.matches - hit.excerpts.length })}</p>
          )}
        </section>
      ))}
    </>
  );
}

function Dashboard({ stats, onRecord }: { stats?: Stats; onRecord: () => void }) {
  const t = useT();
  if (!stats) return <p className="mt-10 text-center text-fg-faint">{t("library.loading")}</p>;
  return (
    <div className="max-w-2xl">
      <h2>{t("library.heading")}</h2>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(120px,1fr))] gap-2.5">
        <Tile label={t("library.meeting")} value={String(stats.meetings)} />
        <Tile label={t("library.recorded")} value={formatDuration(stats.total_duration)} />
        <Tile
          label={t("library.last_7")}
          value={`${stats.last_seven_days}`}
          note={formatDuration(stats.last_seven_days_duration)}
        />
        <Tile label={t("library.people")} value={String(stats.people)} />
        <Tile label={t("meeting.no_summary")} value={String(stats.without_summary)} />
      </div>
      <p className="my-4 text-[13px] text-fg-faint">
        {t("library.vault_hint")}
      </p>
      <button type="button" className="rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-accent-fg transition-opacity hover:opacity-90" onClick={onRecord}>
        {t("library.record_new")}
      </button>
    </div>
  );
}

type Stats = NonNullable<LibraryView["stats"]>;

function Tile({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="flex flex-col gap-0.5 rounded-card border border-line bg-bg-soft p-3.5">
      <span className="tabular text-2xl font-semibold tracking-tight">{value}</span>
      <span className="text-[12px] text-fg-dim">{label}</span>
      {note && <span className="text-[11px] text-fg-faint">{note}</span>}
    </div>
  );
}

function MeetingPane({
  detail,
  folders,
  busy,
  onRename,
  onMove,
  onTags,
  onTrash,
}: {
  detail: MeetingDetail;
  folders: string[];
  busy: boolean;
  onRename: (title: string) => void;
  onMove: (folder: string) => void;
  onTags: (tags: string[]) => void;
  onTrash: () => void;
}) {
  const t = useT();
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
  const known = useMemo(() => [...new Set([...folders, summary.folder])].sort(), [folders, summary.folder]);

  return (
    <article className="max-w-3xl" data-testid="meeting">
      <header className="flex items-start gap-3">
        <input
          className="w-full border-0 border-b border-transparent bg-transparent px-0 py-0.5 text-[22px] font-semibold tracking-tight text-fg hover:border-line focus:border-accent focus:outline-none"
          value={title}
          aria-label={t("meeting.title_label")}
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title !== summary.title && onRename(title.trim())}
        />
        <p className="my-1 mb-3.5 text-[13px] text-fg-faint">
          {dayLabel(summary.day, localDay(), words)} · {timeOfDay(summary.date)} ·{" "}
          {formatDuration(summary.duration)}
          {detail.audio.length > 0 && ` · ${t("library.recordings", { count: detail.audio.length })}`}
        </p>

        <div className="flex flex-wrap items-center gap-3.5 border-b border-line pb-4">
          <label>{t("library.by_folder")}<select
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
          <label>{t("library.by_tag")}<input
              value={tags}
              disabled={busy}
              aria-label={t("library.by_tag")}
              placeholder="weekly, product"
              onChange={(e) => setTags(e.target.value)}
              onBlur={() => onTags(tags.split(",").map((t) => t.trim()).filter(Boolean))}
            />
          </label>
          {confirming ? (
            <span className="flex items-center gap-1.5 text-[13px] text-fg-dim">
              {t("library.trash_confirm")}
              <button
                type="button"
                onClick={onTrash}
                disabled={busy}
                className="rounded-md border border-rec px-2.5 py-1 text-[13px] text-rec hover:bg-rec-soft disabled:opacity-50"
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
        {detail.transcript.length === 0 && <p className="mt-10 text-center text-fg-faint">{t("library.no_content")}</p>}
        <ol className="m-0 list-none p-0">
          {detail.transcript.map((segment) => (
            <li key={segment.seq}>
              <span className="tabular mr-1.5 text-[11px] text-fg-faint">{timestamp(segment.t0)}</span>
              <b>{segment.speaker ?? "?"}</b>
              <span>{segment.text}</span>
            </li>
          ))}
        </ol>
      </section>
    </article>
  );
}
