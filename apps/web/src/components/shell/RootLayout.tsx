import { useMatchRoute, useNavigate, useRouterState } from "@tanstack/react-router";
import type { LucideIcon } from "lucide-react";
import {
  AudioLines,
  Bot,
  CalendarDays,
  ChartNoAxesColumn,
  Library,
  LifeBuoy,
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
  Square,
  Sun,
  Moon,
  Monitor,
  Languages,
} from "lucide-react";
import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { cn } from "../../lib/cn";

import { useI18n, useT } from "../../i18n/context";
import { NoteClient } from "../../lib/notes";
import type { Page } from "./Sidebar";
import { useIsNarrow } from "../../lib/breakpoint";
import { useEngine } from "../../lib/engine-context";
import { deviceWarning } from "../../lib/session";
import { ISSUES } from "../../lib/menu";
import { inShell, isMac } from "../../lib/shell";

/**
 * Both are fetched when they are first needed, and neither ever is on the web build.
 *
 * The menu bar pulls in a dropdown primitive — twenty kilobytes gzipped — for a bar that only the
 * Windows and Linux shells draw. Imported directly it went into the chunk every browser parses
 * before the first paint, including the ones that will never render it.
 */
const MenuBar = lazy(() => import("./MenuBar").then((m) => ({ default: m.MenuBar })));
const Shortcuts = lazy(() => import("./Shortcuts").then((m) => ({ default: m.Shortcuts })));
import { useErrorText } from "../../lib/errors";
import * as sidebar from "../../lib/sidebar";
import { RecordButton } from "../RecordButton";
import { ListeningIn } from "../record/ListeningIn";
import { StatusBar } from "../StatusBar";
import { Waveform } from "../Waveform";
import { m } from "motion/react";

import { SNAPPY, screen as screenVariants } from "../../lib/motion";
import { AssistantPanel } from "../assistant/AssistantPanel";
import { Palette } from "../search/Palette";
import type { Action } from "../../lib/palette";
import { SCHEMES, choose as chooseScheme, read as readScheme, type Scheme } from "../../lib/theme";
import { usePaletteShortcut } from "../../lib/use-palette-shortcut";
import { FirstRun } from "../onboarding/FirstRun";
import { AppShell } from "./AppShell";
import { WindowControls } from "./WindowControls";
import { NudgeBar } from "./NudgeBar";
import { Unreachable } from "./Unreachable";

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
  // Last, because it is where somebody goes when something else did not work — and it has to be
  // somewhere they can see, rather than only behind a menu two of the three platforms draw
  // differently.
  { key: "/help", labelKey: "nav.help", icon: LifeBuoy, group: "setup" },
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
  const { languages, setLocale, locale } = useI18n();
  // Read once. The document already carries the choice — `index.html` applies it before the first
  // paint — so this is only what the palette needs in order to say which one is on.
  const [scheme, setScheme] = useState<Scheme>(() => readScheme());
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
  const [columnOpen, setColumnOpen] = useState(sidebar.wasOpen);
  const [sheetOpen, setSheetOpen] = useState(false);
  // Written on change rather than inside a wrapped setter, because the setter is handed to callers
  // that pass an updater — `setNavOpen((was) => !was)` — and a wrapper would have to reimplement
  // that to know what it was writing.
  useEffect(() => {
    sidebar.remember(columnOpen);
  }, [columnOpen]);
  const navOpen = narrow ? sheetOpen : columnOpen;
  const setNavOpen = narrow ? setSheetOpen : setColumnOpen;
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  /**
   * Whether this window has to draw its own menu bar.
   *
   * Only inside the desktop shell, and not on macOS — there the system draws one at the top of the
   * screen. Read once: neither the platform nor the presence of a shell changes while the app is
   * open, and an effect for a constant is an effect that runs on every render for nothing.
   */
  const [ownMenuBar] = useState(() => inShell() && !isMac());
  /**
   * The page-maker, reachable from a handler defined above it.
   *
   * `newPage` closes over half this component and is declared two hundred lines further down. A ref
   * is the cheap way for an event handler to reach the current one without reordering the file or
   * putting the menu handler's definition somewhere it does not belong.
   */
  const newPageRef = useRef<((where: { folder: string | null }) => void) | null>(null);
  const [assistantOpen, setAssistantOpen] = useState(false);
  usePaletteShortcut(useCallback(() => setPaletteOpen(true), []));

  /**
   * What a menu item means.
   *
   * One handler for both menus — the system's at the top of a Mac's screen, and the app's own in
   * the window everywhere else — because the shell deliberately does not know what a library is.
   * It emits the id it was given and this decides.
   */
  const onMenu = useCallback(
    (id: string) => {
      switch (id) {
        case "new-note":
          // At the root of the vault, which is where a page made from a menu belongs: the menu has
          // no folder selected and inventing one would file somebody's note somewhere they did not
          // choose.
          newPageRef.current?.({ folder: null });
          break;
        case "import":
          void navigate({ to: "/record", search: { source: "upload" } as const });
          break;
        case "record":
          window.dispatchEvent(new CustomEvent("summo:toggle-record"));
          break;
        case "vault":
          void navigate({ to: "/settings", search: { section: "storage" } as const });
          break;
        case "home":
          void navigate({ to: "/" });
          break;
        case "library":
          void navigate({ to: "/library", search: {} });
          break;
        case "tasks":
          void navigate({ to: "/tasks" });
          break;
        case "analytics":
          void navigate({ to: "/analytics" });
          break;
        case "settings":
          void navigate({ to: "/settings", search: {} });
          break;
        case "search":
          setPaletteOpen(true);
          break;
        case "sidebar":
          setNavOpen((open) => !open);
          break;
        case "shortcuts":
          setShortcutsOpen(true);
          break;
        case "docs":
          // Inside the app, not a browser tab. The README is a button at the bottom of it, which is
          // the right order: the person asking "where is my data" wants a paragraph and a link to
          // the storage screen, not a repository.
          void navigate({ to: "/help" });
          break;
        case "issue":
          window.open(ISSUES, "_blank", "noopener,noreferrer");
          break;
        default:
          break;
      }
    },
    [navigate, setNavOpen],
  );

  // The menu bar's events arrive as DOM events — `lib/shell.ts` translates them — and `?` opens the
  // sheet from the keyboard, which is what every app with one uses. Not while somebody is typing:
  // a question mark belongs in the note they are writing.
  useEffect(() => {
    const onMenuEvent = (event: Event) => onMenu(String((event as CustomEvent<string>).detail));
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const typing =
        target?.isContentEditable ||
        ["INPUT", "TEXTAREA", "SELECT"].includes(target?.tagName ?? "");
      if (event.key === "?" && !typing && !event.metaKey && !event.ctrlKey) {
        event.preventDefault();
        setShortcutsOpen(true);
      }
    };
    window.addEventListener("summo:menu", onMenuEvent);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("summo:menu", onMenuEvent);
      window.removeEventListener("keydown", onKey);
    };
  }, [onMenu]);
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
              parent: entry.parent ?? null,
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
      // One address, both kinds. The tree lists a recording and a typed note as the same thing
      // because the vault stores them as the same thing, and until now that promise ended at the
      // click: the two opened different screens, and the caller had to know which.
      void navigate({ to: "/pages/$pageId", params: { pageId: page.id } });
    },
    [navigate],
  );

  const newPage = useCallback(
    (where: { folder: string | null } | { parent: string }) => {
      void (async () => {
        try {
          const parent = "parent" in where ? where.parent : null;
          const { id } = await new NoteClient(engine.handshake).create(
            t("notes.untitled"),
            "",
            parent,
          );
          // A folder or a parent, never both. Filing a sub-page would be answering a question the
          // user did not ask: they said which page it belongs to, not which directory.
          if ("folder" in where && where.folder) await engine.library.moveTo(id, where.folder);
          setVaultGeneration((n) => n + 1);
          void navigate({ to: "/pages/$pageId", params: { pageId: id } });
        } catch (e) {
          // Nothing to show it in up here; the tree simply does not gain a row, and the notes
          // screen is one click away.
          console.warn(say(e));
        }
      })();
    },
    [engine, navigate, say, t],
  );

  // Set after it is declared, read by the menu handler declared above it. See `newPageRef`.
  useEffect(() => {
    newPageRef.current = newPage;
  }, [newPage]);

  const nestPage = useCallback(
    (page: Page, parent: string | null) => {
      // Optimistic, for the same reason a move is: the tree is what the user is looking at while
      // they let go of the mouse, and a row that stays put for a round trip reads as a drop that
      // did not take.
      setPages((current) =>
        current.map((each) => (each.id === page.id ? { ...each, parent } : each)),
      );
      void (async () => {
        try {
          await engine.library.nestUnder(page.id, parent);
        } catch (e) {
          console.warn(say(e));
        } finally {
          // Either way. On failure — a loop the daemon refused — this is what puts the row back
          // where it really is rather than leaving a lie on screen.
          setVaultGeneration((n) => n + 1);
        }
      })();
    },
    [engine.library, say],
  );

  const movePage = useCallback(
    (page: Page, folder: string) => {
      // Optimistic. A move is a rename on disk and comes back in a few milliseconds, but the tree
      // is the thing the user is looking at while they let go of the mouse — a row that stays put
      // for a round trip reads as a drop that did not take.
      setPages((current) =>
        current.map((each) => (each.id === page.id ? { ...each, folder } : each)),
      );
      void (async () => {
        try {
          await engine.library.moveTo(page.id, folder);
        } catch (e) {
          console.warn(say(e));
        } finally {
          // Either way: on success this picks up a folder the vault created, and on failure it puts
          // the row back where it really is rather than leaving a lie on screen.
          setVaultGeneration((n) => n + 1);
        }
      })();
    },
    [engine.library, say],
  );

  /**
   * What ⌘K can do, as opposed to where it can go.
   *
   * Assembled here because every one of them touches state this component owns — the recording, the
   * assistant panel, the shape of the window. Handing them down is what lets the palette stay a
   * list that runs closures instead of a component that has to know what a recording is.
   *
   * Keywords in English beside the Vietnamese labels for the same reason the destinations carry
   * them: the interface language is not the language a person's fingers default to.
   */
  const actions: (Action & { icon: LucideIcon })[] = useMemo(
    () => [
      {
        kind: "action",
        id: "record",
        icon: engine.session.recording ? Square : Mic,
        label: engine.session.recording ? t("record.stop") : t("record.start"),
        keywords: ["record", "ghi", "thu", "stop", "dung"],
        run: () => engine.toggle(),
      },
      {
        kind: "action",
        id: "new-page",
        icon: NotebookPen,
        label: t("nav.new_page"),
        keywords: ["new", "note", "moi", "ghi chu", "trang"],
        run: () => newPage({ folder: search.folder ?? null }),
      },
      {
        kind: "action",
        id: "assistant",
        icon: Sparkles,
        label: t("assistant.title"),
        keywords: ["assistant", "agent", "ask", "tro ly", "hoi"],
        run: () => setAssistantOpen(true),
      },
      {
        kind: "action",
        id: "compact",
        icon: Minimize2,
        label: t("nav.shrink"),
        keywords: ["compact", "shrink", "thu gon", "cua so"],
        run: () => setCompact(true),
      },
      {
        kind: "action",
        id: "sidebar",
        icon: Menu,
        label: navOpen ? t("nav.hide_sidebar") : t("nav.show_sidebar"),
        keywords: ["sidebar", "thanh ben", "an", "hien"],
        run: () => setNavOpen((was) => !was),
      },
      // Three rows rather than one that cycles. A toggle has to be pressed an unknown number of
      // times to reach the state you want, and "the third one is system" is not something a person
      // should have to learn from a palette.
      ...SCHEMES.filter((one) => one !== scheme).map((one) => ({
        kind: "action" as const,
        id: `theme-${one}`,
        icon: { system: Monitor, light: Sun, dark: Moon }[one],
        label: t(`theme.${one}`),
        // Per scheme, not one list shared by all three. A shared list means every theme row matches
        // every theme word, so typing `toi` offers "sáng" as well and Enter picks the wrong one —
        // a filter that returns everything is a filter that has been turned off.
        keywords: {
          system: ["theme", "system", "giao dien", "he thong", "che do"],
          light: ["theme", "light", "giao dien", "sang"],
          dark: ["theme", "dark", "night", "giao dien", "toi"],
        }[one],
        run: () => {
          chooseScheme(one);
          setScheme(one);
        },
      })),
      // Every language but the one already on. Labelled in its own language, because somebody
      // looking for English is not reading the Vietnamese word for it.
      ...languages
        .filter((one) => one.code !== locale)
        .map((one) => ({
          kind: "action" as const,
          id: `locale-${one.code}`,
          icon: Languages,
          label: one.label,
          keywords: ["language", "ngon ngu", one.code],
          run: () => setLocale(one.code),
        })),
    ],
    [engine, languages, locale, newPage, navOpen, scheme, search.folder, setLocale, setNavOpen, t],
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

  // A page is filed in the vault, and `Kho` is the shelf. Neither kind gets its own destination
  // highlighted, because the tree below is what says where the open page lives.
  const active = matchRoute({ to: "/pages/$pageId", fuzzy: true }) ? "/library" : pathname;
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
        // The top inset, for the phone: Android draws this app edge to edge, so without it the
        // status bar's clock and battery are painted over the title and the record button. Zero on
        // a desktop, where there is no inset to clear.
        className="drag-region border-line flex items-center gap-3 border-b px-3 py-2.5 pt-[max(0.625rem,env(safe-area-inset-top))]"
      >
        {/* Windows and Linux hang the menu off the window frame, and this window has none. Drawn
            here, next to the app's name, which is where those platforms put it. */}
        {ownMenuBar && (
          <Suspense fallback={null}>
            <MenuBar onChoose={onMenu} />
          </Suspense>
        )}
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
          {/* Last, and only in the desktop app: the window has no system title bar, so these are
              the only way to minimise it or put it away. Nothing in a browser. */}
          <WindowControls />
        </div>
      </header>

      {/* Above the nudges: "the app cannot reach the thing it runs on" outranks every suggestion
          the app might have about what to do next. */}
      <Unreachable />

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
          onMovePage={movePage}
          onNestPage={nestPage}
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
              <m.div
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
              </m.div>
              {assistantOpen && (
                <div className={cn("h-full shrink-0", narrow ? "w-full" : "w-[380px]")}>
                  <AssistantPanel onClose={() => setAssistantOpen(false)} />
                </div>
              )}
            </div>
          </FirstRun>
        </AppShell>
      </div>
      <Palette open={paletteOpen} onClose={() => setPaletteOpen(false)} actions={actions} />
      {shortcutsOpen && (
        <Suspense fallback={null}>
          <Shortcuts onClose={() => setShortcutsOpen(false)} />
        </Suspense>
      )}
      <StatusBar
        stat={engine.stat}
        speakers={speakersOf(engine.transcript.segments)}
        notice={engine.notice ? say(engine.notice) : null}
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
 * One pattern now that both kinds live at `/pages/<id>`. `?open=` is still read because the notes
 * screen keeps it: it is in links people saved, and a URL that used to work should go on working.
 */
function activePageId(pathname: string, search: { folder?: string; open?: string }): string | null {
  const page = pathname.match(/^\/pages\/([^/]+)/);
  if (page) return page[1] ?? null;
  return search.open ?? null;
}
