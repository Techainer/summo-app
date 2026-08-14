import type { LucideIcon } from "lucide-react";
import { ChevronRight, FileText, Folder, FolderOpen, Mic, Plus } from "lucide-react";
import { motion } from "motion/react";
import { useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { ancestorsOf, buildTree, visibleRows } from "../../lib/folders";

export interface NavItem {
  key: string;
  label: string;
  /**
   * Which band of the sidebar this belongs to.
   *
   * Eleven flat destinations is a list nobody reads, and the flatness said something untrue about
   * the product: recording, reading, tasks and analytics are the work, while voices, agents and
   * models are the machinery that makes the work possible. Two bands say which is which without a
   * word of explanation.
   */
  group?: "work" | "setup";
  /**
   * A drawn icon, not a character.
   *
   * These were typographic glyphs — `●`, `▤`, `◫`, `◔` — picked because they were free. They also
   * render in whatever face the system falls back to, at whatever weight and baseline that face
   * decided, so the column of them was ragged and each one looked like a different piece of
   * punctuation rather than a set. A stroked icon at a fixed size is one weight, one grid, one
   * optical centre.
   */
  icon: LucideIcon;
  /** Unread or outstanding count, e.g. overdue tasks. Zero renders nothing. */
  badge?: number;
}

interface Props {
  items: NavItem[];
  active: string;
  onNavigate: (key: string) => void;
  folders: string[];
  activeFolder: string | null;
  onSelectFolder: (folder: string | null) => void;
  /**
   * Every page in the vault, of both kinds.
   *
   * A recording and a typed note are the same object here, and the sidebar is where that has to be
   * visible: the tree is the user's own structure, and splitting it into "meetings over there,
   * notes over here" would put the app's idea of a document above theirs.
   */
  pages: Page[];
  /** Open a page — the id is all the caller needs; the kind decides which screen. */
  onOpenPage: (page: Page) => void;
  /** The page being read, so the tree shows where you are. */
  activePage?: string | null;
  /** Make a new page, in this folder. `null` is the vault root. */
  onNewPage: (folder: string | null) => void;
  /** Rendered at the bottom: recording controls on desktop, nothing on a sheet. */
  footer?: React.ReactNode;
}

/**
 * The left column: a fixed `Menu` group, then the vault's own folders.
 *
 * The split is the one thing worth copying wholesale from the Coreto reference. Screens are the
 * app's structure and never change; folders are the user's structure and change constantly. Mixing
 * them into one list means the user's own folders keep moving as the app grows.
 */
export function Sidebar({
  items,
  active,
  onNavigate,
  folders,
  activeFolder,
  onSelectFolder,
  pages,
  onOpenPage,
  activePage,
  onNewPage,
  footer,
}: Props) {
  const t = useT();
  const tree = useMemo(() => buildTree(folders), [folders]);
  // Open the path to whatever is selected, so a deep folder is not hidden after a reload.
  const [open, setOpen] = useState<Set<string>>(
    () => new Set(activeFolder ? ancestorsOf(activeFolder) : []),
  );
  const rows = useMemo(() => visibleRows(tree, open), [tree, open]);

  const toggle = (path: string) =>
    setOpen((previous) => {
      const next = new Set(previous);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  return (
    <div className="flex h-full flex-col">
      <nav aria-label={t("nav.screens")} className="px-3 pt-3">
        <p className="text-fg-faint text-micro px-2 pb-1.5 font-semibold tracking-wider uppercase">
          {t("nav.menu")}
        </p>
        <ul className="space-y-0.5">
          {items
            .filter((item) => (item.group ?? "work") === "work")
            .map((item) => (
              <li key={item.key}>
                <NavButton item={item} active={active === item.key} onNavigate={onNavigate} />
              </li>
            ))}
        </ul>

        {/* The machinery, below a rule and a quieter label.
        
            Eleven flat destinations is a list nobody reads, and the flatness also said something
            untrue: recording, reading, tasks and analytics are the work, while voices, agents and
            models are what makes the work possible. Two bands say which is which without a word of
            explanation. */}
        {items.some((item) => item.group === "setup") && (
          <>
            <p className="text-fg-faint text-micro px-2 pt-4 pb-1.5 font-semibold tracking-wider uppercase">
              {t("nav.setup")}
            </p>
            <ul className="space-y-0.5">
              {items
                .filter((item) => item.group === "setup")
                .map((item) => (
                  <li key={item.key}>
                    <NavButton item={item} active={active === item.key} onNavigate={onNavigate} />
                  </li>
                ))}
            </ul>
          </>
        )}
      </nav>

      <div className="border-line mt-4 border-t" />

      <nav aria-label={t("nav.folders")} className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <p className="text-fg-faint text-micro px-2 pb-1.5 font-semibold tracking-wider uppercase">
          {t("nav.folders")}
        </p>
        <ul className="space-y-0.5">
          <li>
            <FolderRow
              label={t("nav.all_folders")}
              depth={0}
              selected={activeFolder === null}
              onSelect={() => onSelectFolder(null)}
            />
          </li>
          {/* Pages filed nowhere sit at the top level, the way an unfiled page does in Notion.
              Hiding them until somebody files them would hide the ones just recorded. */}
          {pagesIn(pages, "").map((page) => (
            <li key={page.id}>
              <PageRow
                page={page}
                depth={1}
                selected={activePage === page.id}
                onOpen={() => onOpenPage(page)}
              />
            </li>
          ))}
          {rows.map((node) => (
            <li key={node.path}>
              <FolderRow
                label={node.name}
                depth={node.depth}
                selected={activeFolder === node.path}
                expandable={node.children.length > 0 || pagesIn(pages, node.path).length > 0}
                expanded={open.has(node.path)}
                onToggle={() => toggle(node.path)}
                onSelect={() => onSelectFolder(node.path)}
                onAdd={() => onNewPage(node.path)}
              />
              {open.has(node.path) && (
                <ul className="space-y-0.5">
                  {pagesIn(pages, node.path).map((page) => (
                    <li key={page.id}>
                      <PageRow
                        page={page}
                        depth={node.depth + 1}
                        selected={activePage === page.id}
                        onOpen={() => onOpenPage(page)}
                      />
                    </li>
                  ))}
                </ul>
              )}
            </li>
          ))}
          {rows.length === 0 && pages.length === 0 && (
            <li className="text-fg-faint text-meta px-2 py-1">{t("nav.no_folders")}</li>
          )}
          <li>
            <button
              type="button"
              onClick={() => onNewPage(activeFolder)}
              className="text-fg-faint hover:bg-bg-soft hover:text-fg mt-1 flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-sm"
            >
              <Plus className="size-3.5" aria-hidden="true" />
              {t("nav.new_page")}
            </button>
          </li>
        </ul>
      </nav>

      {footer && <div className="border-line border-t p-3">{footer}</div>}
    </div>
  );
}

/**
 * One page in the vault.
 *
 * A recording and a typed note differ by one field. That is the whole model: a meeting is a note
 * that happens to have audio and a transcript attached, which is why they live in one tree and are
 * counted, searched and filed by the same code.
 */
export interface Page {
  id: string;
  title: string;
  folder: string;
  kind: "meeting" | "note";
}

/** Pages filed directly in a folder — not in its children, which have their own rows. */
function pagesIn(pages: Page[], folder: string): Page[] {
  return pages.filter((page) => (page.folder ?? "") === folder);
}

function PageRow({
  page,
  depth,
  selected,
  onOpen,
}: {
  page: Page;
  depth: number;
  selected: boolean;
  onOpen: () => void;
}) {
  const Glyph = page.kind === "meeting" ? Mic : FileText;
  return (
    <button
      type="button"
      onClick={onOpen}
      style={{ paddingLeft: `${depth * 12 + 8}px` }}
      className={cn(
        "flex w-full items-center gap-1.5 rounded-lg py-1 pe-2 text-start text-sm transition-colors",
        selected ? "bg-bg-soft text-fg font-medium" : "text-fg-dim hover:bg-bg-soft",
      )}
    >
      {/* The icon is the only thing that says which kind this is, and that is enough: the
          difference matters when you are looking for a recording, and never otherwise. */}
      <Glyph className="text-fg-faint size-3.5 shrink-0" aria-hidden="true" />
      <span className="truncate">{page.title}</span>
    </button>
  );
}

/** `Folder` or `FolderOpen`, so the row says which one the list is showing. */
function FolderIcon({ open, ...rest }: { open: boolean; className?: string }) {
  const Glyph = open ? FolderOpen : Folder;
  return <Glyph {...rest} />;
}

function FolderRow({
  label,
  depth,
  selected,
  expandable = false,
  expanded = false,
  onToggle,
  onSelect,
  onAdd,
}: {
  label: string;
  depth: number;
  selected: boolean;
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  onSelect: () => void;
  /** Make a page here. Absent on the "all folders" row, which is not a place. */
  onAdd?: () => void;
}) {
  const t = useT();
  return (
    <div
      className={cn(
        "flex items-center rounded-lg text-sm transition-colors",
        selected ? "bg-bg-soft text-fg font-medium" : "text-fg-dim hover:bg-bg-soft",
      )}
      // Indent by nesting level. Padding rather than margin so the hover target stays full width.
      style={{ paddingLeft: `${depth * 12}px` }}
    >
      {expandable ? (
        <button
          type="button"
          onClick={onToggle}
          aria-label={t(expanded ? "nav.collapse" : "nav.expand", {
            name: label,
          })}
          aria-expanded={expanded}
          className="text-fg-faint hover:text-fg flex size-6 shrink-0 items-center justify-center"
        >
          <ChevronRight
            aria-hidden="true"
            className={cn("size-3.5 transition-transform duration-150", expanded && "rotate-90")}
          />
        </button>
      ) : (
        <span className="size-6 shrink-0" aria-hidden="true" />
      )}
      {/* Open when it is the folder being browsed. The chevron says whether the *tree* is open;
          this says which folder the list is showing, and they are different facts. */}
      <FolderIcon
        aria-hidden="true"
        className={cn(
          "mr-1.5 size-4 shrink-0 stroke-[1.75]",
          selected ? "text-accent" : "text-fg-faint",
        )}
        open={selected}
      />
      <button
        type="button"
        onClick={onSelect}
        className="flex-1 truncate py-1.5 pr-2 text-left"
        title={label}
      >
        {label}
      </button>
      {onAdd && (
        <button
          type="button"
          onClick={onAdd}
          aria-label={t("nav.new_page_in", { name: label })}
          className="text-fg-faint hover:text-fg flex size-6 shrink-0 items-center justify-center"
        >
          <Plus className="size-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

/**
 * One destination.
 *
 * Extracted so the two bands render the same row rather than two copies that drift — the thing a
 * second list in a sidebar reliably becomes.
 */
function NavButton({
  item,
  active,
  onNavigate,
}: {
  item: NavItem;
  active: boolean;
  onNavigate: (key: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onNavigate(item.key)}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex w-full items-center gap-2.5 rounded-[var(--radius-pill)] px-2.5 py-2 text-sm transition-colors",
        active ? "text-accent font-medium" : "text-fg-dim hover:bg-bg-raised hover:text-fg",
      )}
    >
      {/* The highlight is one element that moves between rows rather than a background that blinks
          on and off. `layoutId` makes the browser interpolate it, so changing screen reads as the
          selection travelling down the list — which is the thing that tells you the list is one
          list. */}
      {active && (
        <motion.span
          layoutId="nav-active"
          aria-hidden="true"
          transition={{ type: "spring", stiffness: 420, damping: 34 }}
          className="bg-accent-soft ring-accent/20 absolute inset-0 -z-10 rounded-[var(--radius-pill)] ring-1"
        />
      )}
      {/* `size-4` and `stroke-[1.75]`: the same optical weight as the label beside it. A 2px stroke
          at this size reads heavier than the text and pulls the eye to the icon rather than to the
          word, which is the wrong way round in a nav. */}
      <item.icon aria-hidden="true" className="size-4 shrink-0 stroke-[1.75]" />
      {item.label}
      {item.badge ? (
        <span className="tabular bg-rec text-micro ml-auto rounded-full px-1.5 py-0.5 font-semibold text-white">
          {item.badge}
        </span>
      ) : null}
    </button>
  );
}
