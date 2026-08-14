import { useMatchRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import type { LucideIcon } from "lucide-react";
import {
  AudioLines,
  Bot,
  CalendarDays,
  ChartNoAxesColumn,
  Library,
  ListChecks,
  Maximize2,
  Menu,
  MessageCircleQuestion,
  House,
  Mic,
  Package,
  Search,
  Sparkles,
  Minimize2,
  NotebookPen,
  Settings,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { cn } from "../../lib/cn";

import { useT } from "../../i18n/context";
import { NoteClient } from "../../lib/notes";
import type { Page } from "./Sidebar";
import { useIsNarrow } from "../../lib/breakpoint";
import { useEngine } from "../../lib/engine-context";
import { deviceWarning } from "../../lib/session";
import { useErrorText } from "../../lib/errors";
import { RecordButton } from "../RecordButton";
import { ListeningIn } from "../record/ListeningIn";
import { StatusBar } from "../StatusBar";
import { Waveform } from "../Waveform";
import { motion } from "motion/react";

import { SNAPPY, screen as screenVariants } from "../../lib/motion";
import { AssistantPanel } from "../assistant/AssistantPanel";
import { Palette } from "../search/Palette";
import { usePaletteShortcut } from "../../lib/use-palette-shortcut";
import { FirstRun } from "../onboarding/FirstRun";
import { AppShell } from "./AppShell";
import { NudgeBar } from "./NudgeBar";

/// Labels are translation keys, resolved at render — a module-level array is built once, before a
/// provider exists, so baking the text in here would freeze the first language forever.
///
/// The icons were typographic characters: `●`, `▤`, `◫`, `◔`. They were free, and they cost the
/// interface more than they saved — each rendered in whatever face the system fell back to, at that
/// face's weight and baseline, so a column of ten looked like ten pieces of punctuation from
/// different books rather than one set.
/// Two bands: the work, then the machinery that makes it possible.
///
/// This was eleven flat items, and the flatness said something untrue about the product. `Thư viện`
/// sat second and listed meetings only, so the navigation claimed Summo was a meeting recorder —
/// while the vault has always held typed notes beside recordings and the daemon has always returned
/// both. `Kho` is that shelf, with a control for which kind you want to see.
const NAV: { key: string; labelKey: string; icon: LucideIcon; group?: "work" | "setup" }[] = [
  { key: "/", labelKey: "nav.home", icon: House },
  { key: "/record", labelKey: "nav.record", icon: Mic },
  { key: "/library", labelKey: "nav.library", icon: Library },
  { key: "/tasks", labelKey: "nav.tasks", icon: ListChecks },
  { key: "/agenda", labelKey: "nav.agenda", icon: CalendarDays },
  { key: "/analytics", labelKey: "nav.analytics", icon: ChartNoAxesColumn },

  { key: "/notes", labelKey: "nav.notes", icon: NotebookPen, group: "setup" },
  { key: "/chat", labelKey: "nav.chat", icon: MessageCircleQuestion, group: "setup" },
  { key: "/people", labelKey: "nav.people", icon: AudioLines, group: "setup" },
  { key: "/agents", labelKey: "nav.agents", icon: Bot, group: "setup" },
  { key: "/models", labelKey: "nav.models", icon: Package, group: "setup" },
  { key: "/settings", labelKey: "nav.settings", icon: Settings, group: "setup" },
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
  const t = useT();
  const navItems = useMemo(
    () =>
      NAV.map((item) => ({
        key: item.key,
        label: t(item.labelKey),
        icon: item.icon,
        group: item.group,
      })),
    [t],
  );
  // Initialised from the breakpoint rather than defaulting open and correcting in an effect: on a
  // phone that would paint the sidebar over the whole app for one frame before closing it.
  // Two states, one per layout, rather than one state pushed back and forth by an effect on every
  // crossing of the breakpoint. A wide window has a column that can be collapsed and starts open; a
  // narrow one has a sheet that starts closed. Kept apart, resizing needs no synchronisation at
  // all, and each layout remembers what the user last did to it.
  const [columnOpen, setColumnOpen] = useState(true);
  const [sheetOpen, setSheetOpen] = useState(false);
  const navOpen = narrow ? sheetOpen : columnOpen;
  const setNavOpen = narrow ? setSheetOpen : setColumnOpen;
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [assistantOpen, setAssistantOpen] = useState(false);
  usePaletteShortcut(useCallback(() => setPaletteOpen(true), []));
  const [compact, setCompact] = useState(false);
  const [folders, setFolders] = useState<string[]>([]);
  const [pages, setPages] = useState<Page[]>([]);
  // Bumped after making a page, so the tree shows it without a reload.
  const [vaultGeneration, setVaultGeneration] = useState(0);
  const say = useErrorText();

  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const search = useRouterState({ select: (s) => s.location.search }) as {
    folder?: string;
    open?: string;
  };

  // The tree comes from the library view: folders, and the pages in them. One request for both,
  // because they are one structure — a folder with no pages under it is a filing cabinet drawn on
  // the wall.
  useEffect(() => {
    let cancelled = false;
    engine.library
      .view({})
      .then((view) => {
        if (cancelled) return;
        setFolders(view.folders);
        setPages(
          view.groups
            .flatMap((group) => group.meetings)
            .map((entry) => ({
              id: entry.id,
              title: entry.title,
              folder: entry.folder ?? "",
              kind: entry.kind,
            })),
        );
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [engine.library, vaultGeneration]);

  const openPage = useCallback(
    (page: Page) => {
      // A meeting and a note are the same object in the vault and open in different screens, which
      // is the one place the difference still shows. Until they share a screen, the tree at least
      // does not make the user care which they are looking for.
      if (page.kind === "meeting") {
        void navigate({ to: `/meetings/${page.id}` });
      } else {
        void navigate({ to: "/notes", search: { open: page.id } });
      }
    },
    [navigate],
  );

  const newPage = useCallback(
    (folder: string | null) => {
      void (async () => {
        try {
          const { id } = await new NoteClient(engine.handshake).create(t("notes.untitled"));
          if (folder) await engine.library.moveTo(id, folder);
          setVaultGeneration((n) => n + 1);
          void navigate({ to: "/notes", search: { open: id } });
        } catch (e) {
          // Nothing to show it in up here; the tree simply does not gain a row, and the notes
          // screen is one click away.
          console.warn(say(e));
        }
      })();
    },
    [engine, navigate, say, t],
  );

  const selectFolder = useCallback(
    (folder: string | null) => {
      // `null` clears the filter; `""` is the vault root, which is a folder like any other. A
      // truthiness test collapses the two and makes "unfiled" impossible to select.
      void navigate({
        to: "/library",
        search: folder === null ? {} : { folder },
      });
    },
    [navigate],
  );

  const active = matchRoute({ to: "/meetings/$meetingId", fuzzy: true }) ? "/library" : pathname;
  const warning = deviceWarning(engine.session);
  const latest = engine.transcript.segments.at(-1);

  if (compact) {
    return (
      <div data-testid="compact" className="drag-region bg-bg flex h-full items-center gap-3 px-3">
        <RecordButton
          recording={engine.session.recording}
          elapsed={engine.elapsed}
          onToggle={engine.toggle}
        />
        <Waveform level={engine.level} active={engine.session.recording} />
        <p className="text-fg-dim text-meta min-w-0 flex-1 truncate">
          {latest?.text ?? t("record.listening")}
        </p>
        <button
          type="button"
          onClick={() => setCompact(false)}
          aria-label={t("nav.expand")}
          title={t("nav.expand_hint")}
          className="text-fg-faint hover:bg-bg-soft hover:text-fg rounded-lg px-2 py-1"
        >
          <Maximize2 aria-hidden="true" className="size-4 stroke-[1.75]" />
        </button>
      </div>
    );
  }

  const header = (
    <>
      <header
        aria-label={t("status.top_bar")}
        className="drag-region border-line flex items-center gap-3 border-b px-3 py-2.5"
      >
        <button
          type="button"
          onClick={() => setNavOpen((open) => !open)}
          aria-label={navOpen ? t("nav.hide_sidebar") : t("nav.show_sidebar")}
          aria-expanded={navOpen}
          className="text-fg-faint hover:bg-bg-soft hover:text-fg rounded-lg px-2 py-1.5"
        >
          <Menu aria-hidden="true" className="size-[18px] stroke-[1.75]" />
        </button>
        <div className="flex items-center gap-2 font-semibold tracking-tight">
          <span className="flex h-4 items-end gap-[2.5px]" aria-hidden="true">
            <i className="bg-accent h-2 w-[3px] rounded-sm" />
            <i className="bg-accent h-4 w-[3px] rounded-sm" />
            <i className="bg-accent h-1.5 w-[3px] rounded-sm" />
          </span>
          Summo
        </div>

        {/* Search where a search bar goes, and the shortcut written on it. Eleven destinations and
            a vault is more than a sidebar can make feel small; this is what does. */}
        <button
          type="button"
          onClick={() => setPaletteOpen(true)}
          aria-label={t("palette.title")}
          className="border-line bg-bg-soft text-fg-faint hover:border-line-strong hover:text-fg-dim text-meta ms-1 hidden items-center gap-2 rounded-[var(--radius-pill)] border px-3 py-1.5 transition-colors sm:flex"
        >
          <Search aria-hidden="true" className="size-3.5" />
          {t("palette.placeholder")}
          <kbd className="text-micro border-line ms-4 rounded border px-1.5 py-0.5">⌘K</kbd>
        </button>
        <div className="ml-auto flex min-w-0 items-center gap-2.5">
          {/* The meter is the first thing to go when there is no room: the record button says the
              same thing, and a two-pixel-wide waveform says nothing. */}
          <span className="hidden sm:flex">
            <Waveform level={engine.level} active={engine.session.recording} />
          </span>
          {/* The assistant, from every screen. It was a destination you navigated away to, which
              is the wrong shape for asking about the thing currently on screen. */}
          <button
            type="button"
            onClick={() => setAssistantOpen((open) => !open)}
            aria-pressed={assistantOpen}
            aria-label={t("assistant.title")}
            title={t("assistant.title")}
            className={cn(
              "rounded-[var(--radius-pill)] px-2 py-1.5 transition-colors",
              assistantOpen ? "bg-ai-soft text-ai" : "text-fg-faint hover:bg-bg-soft hover:text-fg",
            )}
          >
            <Sparkles aria-hidden="true" className="size-4 stroke-[1.75]" />
          </button>
          <RecordButton
            recording={engine.session.recording}
            elapsed={engine.elapsed}
            onToggle={engine.toggle}
          />
          {/* Shrinking the window is a desktop affordance. At phone width there is no window to
              shrink, and keeping it pushed the header 27px past the viewport — the whole app
              scrolled sideways. */}
          <button
            type="button"
            onClick={() => setCompact(true)}
            aria-label={t("nav.shrink")}
            title={t("nav.shrink_hint")}
            className="text-fg-faint hover:bg-bg-soft hover:text-fg hidden rounded-lg px-2 py-1.5 sm:block"
          >
            <Minimize2 aria-hidden="true" className="size-4 stroke-[1.75]" />
          </button>
        </div>
      </header>

      <NudgeBar />

      {/* What the running recording is hearing, on every screen — because a recording survives
          navigation, and "this is in English actually" is realised while looking at something
          else. Renders nothing when idle. */}
      <div className="px-4 empty:hidden [&:has(>*)]:py-2">
        <ListeningIn />
      </div>

      {/* A refused microphone is the one failure with a repair path, so it gets a link to the
          panel that repairs it. Every other failure gets the sentence alone: a button that leads
          somewhere unhelpful is worse than no button. */}
      {engine.session.error && (
        <p className="border-rec/30 bg-rec-soft text-rec text-meta border-b px-4 py-2">
          {say(engine.session.error)}
          {engine.session.error.code === "mic_denied" && (
            <button
              type="button"
              onClick={() => void navigate({ to: "/settings" })}
              className="ml-2 font-medium underline"
            >
              {t("permissions.open")}
            </button>
          )}
        </p>
      )}
      {warning && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta border-b px-4 py-2">
          {warning}
        </p>
      )}
    </>
  );

  return (
    <div className="flex h-full flex-col">
      <div className="min-h-0 flex-1">
        <AppShell
          items={navItems}
          active={active}
          onNavigate={(key) => void navigate({ to: key })}
          folders={folders}
          pages={pages}
          onOpenPage={openPage}
          activePage={activePageId(pathname, search)}
          onNewPage={newPage}
          activeFolder={search.folder ?? null}
          onSelectFolder={selectFolder}
          navOpen={navOpen}
          onNavOpenChange={setNavOpen}
          header={header}
          recording={engine.session.recording}
          onRecord={engine.toggle}
          recordLabel={engine.session.recording ? t("record.stop") : t("record.start")}
        >
          <FirstRun>
            {/* Keyed on the path, so a route change is a change of element and the new screen
                fades in. No `AnimatePresence`, and therefore no exit animation.
                
                It was wrapped in `<AnimatePresence mode="wait">`, and clicking two navigation items
                in quick succession left the whole app blank — the wrapper stuck at the `gone`
                variant, `opacity: 0`, with the screen fully laid out behind it. `mode="wait"` holds
                the incoming child until the outgoing one has finished leaving, and a second key
                change arriving inside that 180 ms window stranded the presence.
                
                Nothing caught it. The browser suites assert on `innerText`, which is happy to
                report text nobody can see, and the screenshot pass reloads the page for every
                shot so it never navigates twice.
                
                The exit was worth four pixels and a fade. Dropping it removes the state machine
                that produced the bug, and takes the original objection with it: with no outgoing
                screen there are never two scroll positions or two sets of tab stops. */}
            {/* Beside the screen, not over it: the two things people do with the assistant are
                asking about the meeting currently open and telling an agent to act while they keep
                reading. Both need the screen to stay visible.
                
                Below the breakpoint there is no room for two columns, so the panel takes the
                whole width — the same component, the same state, one implementation. */}
            <div className="flex h-full min-h-0">
              <motion.div
                key={pathname}
                variants={screenVariants}
                initial="hidden"
                animate="shown"
                transition={SNAPPY}
                className={cn(
                  "h-full min-w-0 flex-1 overflow-y-auto",
                  assistantOpen && narrow && "hidden",
                )}
              >
                {children}
              </motion.div>
              {assistantOpen && (
                <div className={cn("h-full shrink-0", narrow ? "w-full" : "w-[380px]")}>
                  <AssistantPanel onClose={() => setAssistantOpen(false)} />
                </div>
              )}
            </div>
          </FirstRun>
        </AppShell>
      </div>
      <Palette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
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

/**
 * Which page the tree should show as open.
 *
 * A meeting is in the path, a note is in a query parameter, because they still open in different
 * screens. The tree does not care which — it highlights whichever one is on screen.
 */
function activePageId(pathname: string, search: { folder?: string; open?: string }): string | null {
  const meeting = pathname.match(/^\/meetings\/([^/]+)/);
  if (meeting) return meeting[1] ?? null;
  return search.open ?? null;
}
