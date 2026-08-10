import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

const GROUPS: { value: GroupBy; label: string }[] = [
  { value: "day", label: "Ngày" },
  { value: "week", label: "Tuần" },
  { value: "folder", label: "Thư mục" },
];

interface Props {
  client: LibraryClient;
  /** Called when the user wants to record instead of read. */
  onRecord: () => void;
}

/**
 * The library: every meeting already on disk.
 *
 * State is deliberately shallow — a filter change refetches rather than deriving a new view from a
 * cached one. The daemon rescans in a few milliseconds, and a derived view is how a list ends up
 * disagreeing with the files it claims to describe.
 */
export function Library({ client, onRecord }: Props) {
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
      setError(e instanceof Error ? e.message : String(e));
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
        .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
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
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
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
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setBusy(false);
      }
    },
    [client, refresh, selected],
  );

  const stats = view?.stats;

  return (
    <div className="library" data-testid="library">
      <aside className="library-side">
        <div className="search-row">
          <input
            className="search"
            type="search"
            value={query}
            placeholder="Tìm trong mọi cuộc họp…"
            aria-label="Tìm kiếm"
            onChange={(e) => setQuery(e.target.value)}
          />
        </div>

        {hits === null && (
          <div className="segmented" role="group" aria-label="Nhóm theo">
            {GROUPS.map((g) => (
              <button
                key={g.value}
                type="button"
                className={group === g.value ? "on" : ""}
                onClick={() => setGroup(g.value)}
              >
                {g.label}
              </button>
            ))}
          </div>
        )}

        {(folder !== undefined || tag !== undefined) && (
          <div className="chips">
            {folder !== undefined && (
              <button type="button" className="chip on" onClick={() => setFolder(undefined)}>
                📁 {folder === "" ? "Chưa phân loại" : folder} ✕
              </button>
            )}
            {tag !== undefined && (
              <button type="button" className="chip on" onClick={() => setTag(undefined)}>
                #{tag} ✕
              </button>
            )}
          </div>
        )}

        <div className="list" data-testid="meeting-list">
          {hits !== null ? (
            <SearchResults hits={hits} onSelect={setSelected} selected={selected} />
          ) : (
            (view?.groups ?? []).map((g) => (
              <section key={g.key} className="group">
                <h3>{groupLabel(g.key, group, today)}</h3>
                {g.meetings.map((m) => (
                  <MeetingRow
                    key={m.id}
                    meeting={m}
                    selected={m.id === selected}
                    onSelect={() => setSelected(m.id)}
                  />
                ))}
              </section>
            ))
          )}

          {hits === null && view?.total === 0 && (
            <p className="empty">
              Chưa có cuộc họp nào.
              <button type="button" className="link" onClick={onRecord}>
                Ghi buổi đầu tiên
              </button>
            </p>
          )}
          {hits?.length === 0 && <p className="empty">Không tìm thấy “{query}”.</p>}
        </div>

        {view && view.folders.length > 1 && hits === null && (
          <nav className="facets" aria-label="Lọc theo thư mục">
            {view.folders.map((f) => (
              <button key={f} type="button" onClick={() => setFolder(f)}>
                {f === "" ? "Chưa phân loại" : f}
              </button>
            ))}
          </nav>
        )}

        {view && view.tags.length > 0 && hits === null && (
          <nav className="facets" aria-label="Lọc theo thẻ">
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
          <p key={s.path} className="banner warn small">
            Không đọc được {s.path}: {s.reason}
          </p>
        ))}
      </aside>

      <section className="library-main">
        {error && <div className="banner error">{error}</div>}

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
}: {
  meeting: MeetingSummary;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      className={`row${selected ? " on" : ""}`}
      onClick={onSelect}
      aria-current={selected}
    >
      <span className="row-time">{timeOfDay(meeting.date)}</span>
      <span className="row-body">
        <span className="row-title">{meeting.title}</span>
        <span className="row-meta">
          {formatDuration(meeting.duration)}
          {meeting.participants.length > 0 && ` · ${meeting.participants.join(", ")}`}
          {!meeting.has_summary && " · chưa tóm tắt"}
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
  return (
    <>
      {hits.map((hit) => (
        <section key={hit.meeting.id} className="group">
          <MeetingRow
            meeting={hit.meeting}
            selected={hit.meeting.id === selected}
            onSelect={() => onSelect(hit.meeting.id)}
          />
          {hit.excerpts.map((excerpt, i) => (
            <p key={i} className="excerpt">
              {excerpt.t0 !== null && <span className="stamp">{timestamp(excerpt.t0)}</span>}
              {excerpt.speaker && <b>{excerpt.speaker}</b>} {excerpt.text}
            </p>
          ))}
          {hit.matches > hit.excerpts.length && (
            <p className="excerpt more">+{hit.matches - hit.excerpts.length} dòng nữa</p>
          )}
        </section>
      ))}
    </>
  );
}

function Dashboard({ stats, onRecord }: { stats?: Stats; onRecord: () => void }) {
  if (!stats) return <p className="empty">Đang đọc kho họp…</p>;
  return (
    <div className="dashboard">
      <h2>Kho họp của bạn</h2>
      <div className="tiles">
        <Tile label="Cuộc họp" value={String(stats.meetings)} />
        <Tile label="Đã ghi" value={formatDuration(stats.total_duration)} />
        <Tile
          label="7 ngày qua"
          value={`${stats.last_seven_days}`}
          note={formatDuration(stats.last_seven_days_duration)}
        />
        <Tile label="Người" value={String(stats.people)} />
        <Tile label="Chưa tóm tắt" value={String(stats.without_summary)} />
      </div>
      <p className="hint">
        Mọi thứ nằm trong <code>~/.summo/vault</code> — mở bằng Obsidian, grep, hay sao lưu tuỳ bạn.
      </p>
      <button type="button" className="primary" onClick={onRecord}>
        Ghi cuộc họp mới
      </button>
    </div>
  );
}

type Stats = NonNullable<LibraryView["stats"]>;

function Tile({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="tile">
      <span className="tile-value">{value}</span>
      <span className="tile-label">{label}</span>
      {note && <span className="tile-note">{note}</span>}
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

  const known = useMemo(() => [...new Set([...folders, summary.folder])].sort(), [folders, summary.folder]);

  return (
    <article className="meeting" data-testid="meeting">
      <header className="meeting-head">
        <input
          className="title-input"
          value={title}
          aria-label="Tên cuộc họp"
          onChange={(e) => setTitle(e.target.value)}
          onBlur={() => title.trim() && title !== summary.title && onRename(title.trim())}
        />
        <p className="meeting-meta">
          {dayLabel(summary.day, localDay())} · {timeOfDay(summary.date)} ·{" "}
          {formatDuration(summary.duration)}
          {detail.audio.length > 0 && ` · ${detail.audio.length} bản ghi âm`}
        </p>

        <div className="meeting-actions">
          <label>
            Thư mục
            <select
              value={summary.folder}
              disabled={busy}
              onChange={(e) => onMove(e.target.value)}
              aria-label="Thư mục"
            >
              {known.map((f) => (
                <option key={f} value={f}>
                  {f === "" ? "Chưa phân loại" : f}
                </option>
              ))}
            </select>
          </label>
          <label>
            Thẻ
            <input
              value={tags}
              disabled={busy}
              aria-label="Thẻ"
              placeholder="weekly, product"
              onChange={(e) => setTags(e.target.value)}
              onBlur={() => onTags(tags.split(",").map((t) => t.trim()).filter(Boolean))}
            />
          </label>
          {confirming ? (
            <span className="confirm">
              Chuyển vào thùng rác?
              <button type="button" className="danger" onClick={onTrash} disabled={busy}>
                Chuyển
              </button>
              <button type="button" onClick={() => setConfirming(false)}>
                Huỷ
              </button>
            </span>
          ) : (
            <button type="button" className="ghost" onClick={() => setConfirming(true)}>
              Xoá
            </button>
          )}
        </div>
      </header>

      {detail.sections.map((s) => (
        <section key={s.heading} className="meeting-section">
          <h3>{s.heading}</h3>
          <p>{s.body}</p>
        </section>
      ))}

      <section className="meeting-section">
        <h3>Bản ghi ({detail.transcript.length} dòng)</h3>
        {detail.transcript.length === 0 && <p className="empty">Chưa có nội dung.</p>}
        <ol className="lines">
          {detail.transcript.map((segment) => (
            <li key={segment.seq}>
              <span className="stamp">{timestamp(segment.t0)}</span>
              <b>{segment.speaker ?? "?"}</b>
              <span>{segment.text}</span>
            </li>
          ))}
        </ol>
      </section>
    </article>
  );
}
