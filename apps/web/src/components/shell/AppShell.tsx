import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useIsNarrow } from "../../lib/breakpoint";
import { SPRING } from "../../lib/motion";
import { Sheet } from "../ui";
import { BottomBar } from "./BottomBar";
import { Sidebar, type NavItem } from "./Sidebar";

interface Props {
  items: NavItem[];
  active: string;
  onNavigate: (key: string) => void;
  folders: string[];
  pages: import("./Sidebar").Page[];
  onOpenPage: (page: import("./Sidebar").Page) => void;
  activePage?: string | null;
  onNewPage: (folder: string | null) => void;
  onMovePage: (page: import("./Sidebar").Page, folder: string) => void;
  activeFolder: string | null;
  onSelectFolder: (folder: string | null) => void;
  /** Open state of the sidebar: a column on wide screens, a sheet on narrow ones. */
  navOpen: boolean;
  onNavOpenChange: (open: boolean) => void;
  header: ReactNode;
  sidebarFooter?: ReactNode;
  children: ReactNode;
  /** Whether a recording is running, for the button in the bottom bar. */
  recording: boolean;
  onRecord: () => void;
  recordLabel: string;
}

/**
 * The window: a sidebar, a header and the screen itself.
 *
 * The same `Sidebar` renders in both layouts. Below 768 px it moves into a `Sheet` rather than
 * being replaced by a different component, so there is one navigation implementation to keep
 * correct instead of two that drift.
 */
export function AppShell({
  items,
  active,
  onNavigate,
  folders,
  pages,
  onOpenPage,
  activePage,
  onNewPage,
  onMovePage,
  activeFolder,
  onSelectFolder,
  navOpen,
  onNavOpenChange,
  header,
  sidebarFooter,
  children,
  recording,
  onRecord,
  recordLabel,
}: Props) {
  const t = useT();
  const narrow = useIsNarrow();

  // On a sheet, choosing something should close it — the user has finished navigating.
  const navigate = (key: string) => {
    onNavigate(key);
    if (narrow) onNavOpenChange(false);
  };
  const selectFolder = (folder: string | null) => {
    onSelectFolder(folder);
    if (narrow) onNavOpenChange(false);
  };

  const sidebar = (
    <Sidebar
      items={items}
      active={active}
      onNavigate={navigate}
      folders={folders}
      pages={pages}
      onOpenPage={(page) => {
        onOpenPage(page);
        // Opening a page is finishing navigating, same as choosing a screen.
        if (narrow) onNavOpenChange(false);
      }}
      activePage={activePage}
      onNewPage={onNewPage}
      onMovePage={onMovePage}
      activeFolder={activeFolder}
      onSelectFolder={selectFolder}
      footer={narrow ? undefined : sidebarFooter}
    />
  );

  return (
    <div className="flex h-full">
      {/* Collapsed means gone, not clipped. Hiding it with `width: 0` and `overflow: hidden` leaves
          every link in the accessibility tree and reachable with Tab, so a keyboard user tabs
          through a sidebar they cannot see. */}
      <AnimatePresence initial={false}>
        {!narrow && navOpen && (
          <motion.aside
            key="sidebar"
            // A surface of its own, not a translucent wash of the page. The window used to be one
            // flat colour from edge to edge, so the sidebar existed only because of a one-pixel
            // border — which is what made the whole app read as a wireframe rather than a product.
            className="border-line bg-bg-soft shrink-0 overflow-hidden border-r"
            initial={{ width: 0 }}
            animate={{ width: 270 }}
            exit={{ width: 0 }}
            transition={SPRING}
          >
            <div className="h-full w-[270px]">{sidebar}</div>
          </motion.aside>
        )}
      </AnimatePresence>

      {narrow && (
        <Sheet
          open={navOpen}
          onOpenChange={onNavOpenChange}
          side="left"
          title={t("nav.label")}
          description={t("record.pick_screen")}
        >
          {sidebar}
        </Sheet>
      )}

      <div className="flex min-w-0 flex-1 flex-col">
        {header}
        <main className="bg-bg min-h-0 flex-1 overflow-y-auto">{children}</main>
        {/* Below the breakpoint the only way to change screen was the hamburger — in the top-left,
            the hardest corner for a right thumb, behind a full-screen sheet. The four destinations
            that get used move to the bottom where the hand already is; the sheet keeps the rest,
            which is the right place for settings and model management. */}
        {narrow && (
          <BottomBar
            // `/record` is left out on purpose: the raised button in the middle of the bar *is*
            // recording, and a tab beside it going to the same place is two controls for one job.
            items={items.filter(
              (item) => (item.group ?? "work") === "work" && item.key !== "/record",
            )}
            active={active}
            onNavigate={navigate}
            recording={recording}
            onRecord={onRecord}
            recordLabel={recordLabel}
          />
        )}
      </div>
    </div>
  );
}
