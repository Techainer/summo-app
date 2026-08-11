import { useMatchRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useIsNarrow } from "../../lib/breakpoint";
import { useEngine } from "../../lib/engine-context";
import { deviceWarning } from "../../lib/session";
import { RecordButton } from "../RecordButton";
import { StatusBar } from "../StatusBar";
import { Waveform } from "../Waveform";
import { AnimatePresence, motion } from "motion/react";

import { SNAPPY, screen as screenVariants } from "../../lib/motion";
import { FirstRun } from "../onboarding/FirstRun";
import { AppShell } from "./AppShell";
import { NudgeBar } from "./NudgeBar";

/// Labels are translation keys, resolved at render — a module-level array is built once, before a
/// provider exists, so baking the text in here would freeze the first language forever.
const NAV: { key: string; labelKey: string; icon: string }[] = [
  { key: "/", labelKey: "nav.record", icon: "●" },
  { key: "/library", labelKey: "nav.library", icon: "▤" },
  { key: "/notes", labelKey: "nav.notes", icon: "✎" },
  { key: "/agenda", labelKey: "nav.agenda", icon: "◫" },
  { key: "/tasks", labelKey: "nav.tasks", icon: "☑" },
  { key: "/agents", labelKey: "nav.agents", icon: "◈" },
  { key: "/chat", labelKey: "nav.chat", icon: "◇" },
  { key: "/people", labelKey: "nav.people", icon: "◍" },
  { key: "/analytics", labelKey: "nav.analytics", icon: "◔" },
  { key: "/settings", labelKey: "nav.settings", icon: "⚙" },
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
    () => NAV.map((item) => ({ key: item.key, label: t(item.labelKey), icon: item.icon })),
    [t],
  );
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
      // `null` clears the filter; `""` is the vault root, which is a folder like any other. A
      // truthiness test collapses the two and makes "unfiled" impossible to select.
      void navigate({ to: "/library", search: folder === null ? {} : { folder } });
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
          aria-label={t("nav.expand")}
          title={t("nav.expand_hint")}
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
        aria-label={t("status.top_bar")}
        className="drag-region flex items-center gap-3 border-b border-line px-3 py-2.5"
      >
        <button
          type="button"
          onClick={() => setNavOpen((open) => !open)}
          aria-label={navOpen ? t("nav.hide_sidebar") : t("nav.show_sidebar")}
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
            className="hidden rounded-lg px-2 py-1.5 text-fg-faint hover:bg-bg-soft hover:text-fg sm:block"
          >
            ⤡
          </button>
        </div>
      </header>

      <NudgeBar />

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
            {/* Keyed on the path so a route change is a change of element, which is what gives
                `AnimatePresence` something to animate out. `mode="wait"` rather than a cross-fade:
                two screens overlapping mid-transition means two scroll positions and two sets of
                focusable controls, and a keyboard user can tab into the screen that is leaving. */}
            <AnimatePresence mode="wait" initial={false}>
              <motion.div
                key={pathname}
                variants={screenVariants}
                initial="hidden"
                animate="shown"
                exit="gone"
                transition={SNAPPY}
                className="h-full"
              >
                {children}
              </motion.div>
            </AnimatePresence>
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
