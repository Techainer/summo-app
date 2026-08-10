import { AnimatePresence, motion } from "motion/react";
import type { ReactNode } from "react";

import { useT } from "../../i18n/context";
import { useIsNarrow } from "../../lib/breakpoint";
import { Sheet } from "../ui";
import { Sidebar, type NavItem } from "./Sidebar";

interface Props {
  items: NavItem[];
  active: string;
  onNavigate: (key: string) => void;
  folders: string[];
  activeFolder: string | null;
  onSelectFolder: (folder: string | null) => void;
  /** Open state of the sidebar: a column on wide screens, a sheet on narrow ones. */
  navOpen: boolean;
  onNavOpenChange: (open: boolean) => void;
  header: ReactNode;
  sidebarFooter?: ReactNode;
  children: ReactNode;
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
  activeFolder,
  onSelectFolder,
  navOpen,
  onNavOpenChange,
  header,
  sidebarFooter,
  children,
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
            className="shrink-0 overflow-hidden border-r border-line bg-bg-soft/40"
            initial={{ width: 0 }}
            animate={{ width: 270 }}
            exit={{ width: 0 }}
            transition={{ type: "spring", stiffness: 420, damping: 40 }}
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
        <main className="min-h-0 flex-1 overflow-y-auto">{children}</main>
      </div>
    </div>
  );
}
