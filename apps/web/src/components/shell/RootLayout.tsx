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
  Minimize2,
  NotebookPen,
  Settings,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useIsNarrow } from "../../lib/breakpoint";
import { useEngine } from "../../lib/engine-context";
import { deviceWarning } from "../../lib/session";
import { RecordButton } from "../RecordButton";
import { StatusBar } from "../StatusBar";
import { Waveform } from "../Waveform";
import { motion } from "motion/react";

import { SNAPPY, screen as screenVariants } from "../../lib/motion";
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
  const [navOpen, setNavOpen] = useState(() => !narrow);
  const [compact, setCompact] = useState(false);
  const [folders, setFolders] = useState<string[]>([]);

  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const search = useRouterState({ select: (s) => s.location.search }) as {
    folder?: string;
  };

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
        <p className="text-fg-dim min-w-0 flex-1 truncate text-[13px]">
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
        <div className="ml-auto flex min-w-0 items-center gap-2.5">
          {/* The meter is the first thing to go when there is no room: the record button says the
              same thing, and a two-pixel-wide waveform says nothing. */}
          <span className="hidden sm:flex">
            <Waveform level={engine.level} active={engine.session.recording} />
          </span>
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

      {engine.session.error && (
        <p className="border-rec/30 bg-rec-soft text-rec border-b px-4 py-2 text-[13px]">
          {engine.session.error}
        </p>
      )}
      {warning && (
        <p className="border-blocked/30 bg-blocked-soft text-blocked border-b px-4 py-2 text-[13px]">
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
          activeFolder={search.folder ?? null}
          onSelectFolder={selectFolder}
          navOpen={navOpen}
          onNavOpenChange={setNavOpen}
          header={header}
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
            <motion.div
              key={pathname}
              variants={screenVariants}
              initial="hidden"
              animate="shown"
              transition={SNAPPY}
              className="h-full"
            >
              {children}
            </motion.div>
          </FirstRun>
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
