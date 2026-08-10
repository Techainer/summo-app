import { useMatchRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import { useCallback, useEffect, useState, type ReactNode } from "react";

import { useIsNarrow } from "../../lib/breakpoint";
import { useEngine } from "../../lib/engine-context";
import { deviceWarning } from "../../lib/session";
import { RecordButton } from "../RecordButton";
import { StatusBar } from "../StatusBar";
import { Waveform } from "../Waveform";
import { AppShell } from "./AppShell";
import type { NavItem } from "./Sidebar";

const NAV: NavItem[] = [
  { key: "/", label: "Ghi", icon: "●" },
  { key: "/library", label: "Thư viện", icon: "▤" },
  { key: "/tasks", label: "Việc", icon: "☑" },
  { key: "/people", label: "Giọng nói", icon: "◍" },
  { key: "/analytics", label: "Thống kê", icon: "◔" },
  { key: "/settings", label: "Cài đặt", icon: "⚙" },
];

/**
 * Everything that stays on screen as the route changes: sidebar, header, status bar.
 *
 * The recording controls live in the header rather than on the record screen, because a meeting in
 * progress has to be stoppable from wherever the user has navigated to.
 */
export function RootLayout({ children }: { children: ReactNode }) {
  const engine = useEngine();
  const navigate = useNavigate();
  const matchRoute = useMatchRoute();
  const narrow = useIsNarrow();
  // Initialised from the breakpoint rather than defaulting open and correcting in an effect: on a
  // phone that would paint the sidebar over the whole app for one frame before closing it.
  const [navOpen, setNavOpen] = useState(() => !narrow);
  const [compact, setCompact] = useState(false);
  const [folders, setFolders] = useState<string[]>([]);

  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const search = useRouterState({ select: (s) => s.location.search }) as { folder?: string };

  // Crossing the breakpoint by resizing resets the sidebar to that layout's natural state.
  useEffect(() => setNavOpen(!narrow), [narrow]);

  // Folders come from the library view and drive the sidebar tree.
  useEffect(() => {
    let cancelled = false;
    engine.library
      .view({})
      .then((view) => !cancelled && setFolders(view.folders))
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [engine.library]);

  const selectFolder = useCallback(
    (folder: string | null) => {
      void navigate({ to: "/library", search: folder ? { folder } : {} });
    },
    [navigate],
  );

  const active = matchRoute({ to: "/meetings/$meetingId", fuzzy: true }) ? "/library" : pathname;
  const warning = deviceWarning(engine.session);
  const latest = engine.transcript.segments.at(-1);

  if (compact) {
    return (
      <div
        data-testid="compact"
        className="drag-region flex h-full items-center gap-3 bg-bg px-3"
      >
        <RecordButton
          recording={engine.session.recording}
          elapsed={engine.elapsed}
          onToggle={engine.toggle}
        />
        <Waveform level={engine.level} active={engine.session.recording} />
        <p className="min-w-0 flex-1 truncate text-[13px] text-fg-dim">
          {latest?.text ?? "Đang nghe…"}
        </p>
        <button
          type="button"
          onClick={() => setCompact(false)}
          aria-label="Mở rộng cửa sổ"
          title="Mở rộng"
          className="rounded-lg px-2 py-1 text-fg-faint hover:bg-bg-soft hover:text-fg"
        >
          ⤢
        </button>
      </div>
    );
  }

  const header = (
    <>
      <header
        aria-label="Thanh trên cùng"
        className="drag-region flex items-center gap-3 border-b border-line px-3 py-2.5"
      >
        <button
          type="button"
          onClick={() => setNavOpen((open) => !open)}
          aria-label={navOpen ? "Ẩn thanh bên" : "Hiện thanh bên"}
          aria-expanded={navOpen}
          className="rounded-lg px-2 py-1.5 text-fg-faint hover:bg-bg-soft hover:text-fg"
        >
          ☰
        </button>
        <div className="flex items-center gap-2 font-semibold tracking-tight">
          <span className="flex h-4 items-end gap-[2.5px]" aria-hidden="true">
            <i className="h-2 w-[3px] rounded-sm bg-accent" />
            <i className="h-4 w-[3px] rounded-sm bg-accent" />
            <i className="h-1.5 w-[3px] rounded-sm bg-accent" />
          </span>
          Summo
        </div>
        <div className="ml-auto flex items-center gap-2.5">
          <Waveform level={engine.level} active={engine.session.recording} />
          <RecordButton
            recording={engine.session.recording}
            elapsed={engine.elapsed}
            onToggle={engine.toggle}
          />
          <button
            type="button"
            onClick={() => setCompact(true)}
            aria-label="Thu gọn cửa sổ"
            title="Thu gọn khi đang họp"
            className="rounded-lg px-2 py-1.5 text-fg-faint hover:bg-bg-soft hover:text-fg"
          >
            ⤡
          </button>
        </div>
      </header>

      {engine.session.error && (
        <p className="border-b border-rec/30 bg-rec-soft px-4 py-2 text-[13px] text-rec">
          {engine.session.error}
        </p>
      )}
      {warning && (
        <p className="border-b border-blocked/30 bg-blocked-soft px-4 py-2 text-[13px] text-blocked">
          {warning}
        </p>
      )}
    </>
  );

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1">
        <AppShell
          items={NAV}
          active={active}
          onNavigate={(key) => void navigate({ to: key })}
          folders={folders}
          activeFolder={search.folder ?? null}
          onSelectFolder={selectFolder}
          navOpen={navOpen}
          onNavOpenChange={setNavOpen}
          header={header}
        >
          {children}
        </AppShell>
      </div>
      <StatusBar
        stat={engine.stat}
        speakers={speakersOf(engine.transcript.segments)}
        notice={engine.notice}
        connection={engine.session.connection}
        device={engine.session.deviceLabel}
      />
    </div>
  );
}

function speakersOf(segments: { speaker?: string | null }[]): string[] {
  const seen = new Set<string>();
  for (const segment of segments) if (segment.speaker) seen.add(segment.speaker);
  return [...seen];
}
