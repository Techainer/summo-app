import type { LucideIcon } from "lucide-react";
import { ChevronRight, FileText, Folder, FolderOpen, FolderInput, Mic, Plus } from "lucide-react";
import { m } from "motion/react";
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
  /**
   * Make a new page.
   *
   * A folder — `null` for the vault root — or a page to put it inside, never both: those are the
   * two ways of saying where something goes and they are answered by two different gestures.
   */
  onNewPage: (where: { folder: string | null } | { parent: string }) => void;
  /**
   * Put a page inside another, or `null` to take it back out to the top level.
   *
   * The file does not move. The daemon refuses a page nested under one of its own descendants,
   * because a loop in this tree is not a wrong drawing but an infinite one.
   */
  onNestPage: (page: Page, parent: string | null) => void;
  /**
   * File a page somewhere else. `""` is the vault root.
   *
   * The tree is the only place in the app where every page and every folder are visible at once,
   * which makes it the only place filing can be done without first going to find the thing.
   */
  onMovePage: (page: Page, folder: string) => void;
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
  onMovePage,
  onNestPage,
  footer,
}: Props) {
  const t = useT();
  const tree = useMemo(() => buildTree(folders), [folders]);
  // Open the path to whatever is selected, so a deep folder is not hidden after a reload.
  const [open, setOpen] = useState<Set<string>>(
    () => new Set(activeFolder ? ancestorsOf(activeFolder) : []),
  );
  const rows = useMemo(() => visibleRows(tree, open), [tree, open]);

  /**
   * The page being dragged, and the folder the pointer is currently over.
   *
   * Held here rather than read out of `dataTransfer` because the drag data is deliberately
   * unreadable during `dragover` — the browser only hands it over on drop, so a row cannot decide
   * whether to highlight itself from the event alone.
   */
  const [dragging, setDragging] = useState<Page | null>(null);
  /**
   * The row under the pointer, as `folder:<path>` or `page:<id>`.
   *
   * Namespaced because both are drop targets now and they mean different things — dropping on a
   * folder files the page, dropping on a page nests it — and an id and a folder path share a
   * namespace in which the vault root is `""`, which is also a perfectly good id to nobody.
   */
  const [over, setOver] = useState<string | null>(null);
  /** Which page has its destination list open, for pointers that cannot drag. */
  const [filing, setFiling] = useState<string | null>(null);
  /** Which pages are showing what is inside them. */
  const [unfolded, setUnfolded] = useState<Set<string>>(new Set());

  /**
   * Every folder a page could go to, root included.
   *
   * The tree's own rows are not the answer: a collapsed folder is still somewhere you can file
   * something, and a list that only offers what happens to be expanded would make filing depend on
   * what the user last clicked.
   */
  const destinations = useMemo(() => {
    const all = new Set<string>([""]);
    for (const folder of folders) all.add(folder);
    return [...all].sort();
  }, [folders]);

  /**
   * Pages by the page they are inside; `""` holds the ones that are inside nothing.
   *
   * A parent nobody has — a page whose file was deleted, or an id somebody typed into frontmatter
   * by hand — puts its children back at the top level rather than hiding them. Losing a page
   * because the row that would have contained it does not exist is not a failure a person can see
   * their way out of.
   */
  const inside = useMemo(() => {
    const known = new Set(pages.map((page) => page.id));
    const map = new Map<string, Page[]>();
    for (const page of pages) {
      const under = page.parent && known.has(page.parent) ? page.parent : "";
      map.set(under, [...(map.get(under) ?? []), page]);
    }
    return map;
  }, [pages]);

  const toggle = (path: string) =>
    setOpen((previous) => {
      const next = new Set(previous);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  const unfold = (id: string) =>
    setUnfolded((previous) => {
      const next = new Set(previous);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const done = () => {
    setFiling(null);
    setDragging(null);
    setOver(null);
  };

  const file = (page: Page, folder: string) => {
    done();
    if ((page.folder ?? "") === folder) return;
    onMovePage(page, folder);
    // Open the folder it landed in, so the page is visible where it went rather than apparently
    // deleted from where it was.
    if (folder) setOpen((previous) => new Set([...previous, ...ancestorsOf(folder), folder]));
  };

  const nest = (page: Page, parent: string | null) => {
    done();
    if ((page.parent ?? null) === parent) return;
    // Not into itself, and not into anything already inside it. The daemon refuses both — a loop
    // here is not a wrong drawing but an infinite one — and refusing on this side as well is what
    // stops the optimistic row moving somewhere it is about to be moved back from.
    if (parent && (parent === page.id || descendants(inside, page.id).has(parent))) return;
    onNestPage(page, parent);
    if (parent) setUnfolded((previous) => new Set([...previous, parent]));
  };

  /** The handlers that make a row a place a page can be dropped. */
  const dropzone = (key: string, drop: (page: Page) => void) => ({
    onDragOver: (event: React.DragEvent) => {
      if (!dragging) return;
      // Without this the browser refuses the drop and animates the row back to where it came
      // from — the default for every element is "nothing may be dropped here".
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
    },
    onDragEnter: () => dragging && setOver(key),
    // Fires when the pointer crosses into a *child* element too, so the row is only cleared when
    // the pointer has moved on to a different row.
    onDragLeave: () => setOver((at) => (at === key ? null : at)),
    onDrop: (event: React.DragEvent) => {
      event.preventDefault();
      if (dragging) drop(dragging);
    },
  });

  /** Everything one page row and the rows under it need. Bundled, because it recurses. */
  const branch = {
    activePage,
    dragging,
    over,
    filing,
    unfolded,
    inside,
    destinations,
    onOpen: onOpenPage,
    onUnfold: unfold,
    onFiling: (id: string) => setFiling((at) => (at === id ? null : id)),
    onDragStart: setDragging,
    onDragEnd: done,
    onFile: file,
    onNest: nest,
    onAddChild: (parent: string) => onNewPage({ parent }),
    dropzone,
  };

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
            {/* Also where a page goes to stop being filed. The unfiled pages render directly under
                this row, so it is the band the root already occupies; dropping onto it puts the
                page back among them. */}
            <FolderRow
              label={t("nav.all_folders")}
              depth={0}
              selected={activeFolder === null}
              onSelect={() => onSelectFolder(null)}
              dropping={over === "folder:"}
              dropHint={dragging ? t("nav.move_to_root") : undefined}
              {...dropzone("folder:", (page) => file(page, ""))}
            />
          </li>
          {/* Pages filed nowhere sit at the top level, the way an unfiled page does in Notion.
              Hiding them until somebody files them would hide the ones just recorded. */}
          {topLevel(inside, "").map((page) => (
            <li key={page.id}>
              <Branch page={page} depth={1} {...branch} />
            </li>
          ))}
          {rows.map((node) => (
            <li key={node.path}>
              <FolderRow
                label={node.name}
                depth={node.depth}
                selected={activeFolder === node.path}
                expandable={node.children.length > 0 || topLevel(inside, node.path).length > 0}
                expanded={open.has(node.path)}
                onToggle={() => toggle(node.path)}
                onSelect={() => onSelectFolder(node.path)}
                onAdd={() => onNewPage({ folder: node.path })}
                dropping={over === `folder:${node.path}`}
                dropHint={dragging ? t("nav.move_to", { name: node.name }) : undefined}
                {...dropzone(`folder:${node.path}`, (page) => file(page, node.path))}
              />
              {open.has(node.path) && (
                <ul className="space-y-0.5">
                  {topLevel(inside, node.path).map((page) => (
                    <li key={page.id}>
                      <Branch page={page} depth={node.depth + 1} {...branch} />
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
              onClick={() => onNewPage({ folder: activeFolder })}
              className="text-fg-faint hover:bg-bg-raised hover:text-fg mt-1 flex w-full items-center gap-1.5 rounded-lg px-2 py-1 text-sm"
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
  /**
   * The page this one is inside, if it is inside one.
   *
   * A folder and a parent are two structures over the same set and both belong to the user: a
   * folder is where the file *is*, a parent is what the page is *part of*. A page with a parent is
   * drawn under it and nowhere else — including when its file is filed somewhere else entirely,
   * which is allowed and is why nesting does not move anything.
   */
  parent: string | null;
}

/** A page and everything inside it. */
interface BranchProps {
  page: Page;
  depth: number;
  activePage?: string | null;
  dragging: Page | null;
  over: string | null;
  filing: string | null;
  unfolded: Set<string>;
  inside: Map<string, Page[]>;
  destinations: string[];
  onOpen: (page: Page) => void;
  onUnfold: (id: string) => void;
  onFiling: (id: string) => void;
  onDragStart: (page: Page) => void;
  onDragEnd: () => void;
  onFile: (page: Page, folder: string) => void;
  onNest: (page: Page, parent: string | null) => void;
  onAddChild: (parent: string) => void;
  dropzone: (key: string, drop: (page: Page) => void) => Record<string, unknown>;
}

/**
 * One page row, and the pages inside it.
 *
 * Recursive, because the structure is. The alternative — flattening the tree into rows with a depth
 * number, which is what the *folders* beside this do — works for folders because a folder path
 * already spells out its ancestry. A page knows only its parent, so flattening would mean building
 * the tree anyway and then walking it twice.
 *
 * `seen` is the guard on that recursion. The daemon refuses to create a cycle, but frontmatter is a
 * thing people edit by hand, and a vault that already contains `a → b → a` must draw a tree with a
 * missing row rather than lock the tab up.
 */
function Branch({ page, depth, seen = [], ...rest }: BranchProps & { seen?: string[] }) {
  const t = useT();
  const children = seen.includes(page.id) ? [] : (rest.inside.get(page.id) ?? []);
  const open = rest.unfolded.has(page.id);

  return (
    <>
      <PageRow
        page={page}
        depth={depth}
        selected={rest.activePage === page.id}
        onOpen={() => rest.onOpen(page)}
        dragging={rest.dragging?.id === page.id}
        onDragStart={() => rest.onDragStart(page)}
        onDragEnd={rest.onDragEnd}
        filing={rest.filing === page.id}
        onFile={() => rest.onFiling(page.id)}
        destinations={rest.destinations}
        onMoveTo={(folder) => rest.onFile(page, folder)}
        onUnnest={page.parent ? () => rest.onNest(page, null) : undefined}
        children={children.length}
        expanded={open}
        onToggle={() => rest.onUnfold(page.id)}
        onAdd={() => rest.onAddChild(page.id)}
        // Dropping a page onto a page puts it inside — which is the gesture, and the reason the
        // folder rows and these rows have to be told apart by their drop key rather than by a bare
        // string that could be either.
        dropping={rest.over === `page:${page.id}`}
        dropHint={
          rest.dragging && rest.dragging.id !== page.id
            ? t("nav.nest_under", { name: page.title })
            : undefined
        }
        {...rest.dropzone(`page:${page.id}`, (dragged) => rest.onNest(dragged, page.id))}
      />
      {open && children.length > 0 && (
        <ul className="space-y-0.5">
          {children.map((child) => (
            <li key={child.id}>
              <Branch page={child} depth={depth + 1} seen={[...seen, page.id]} {...rest} />
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function PageRow({
  page,
  depth,
  selected,
  onOpen,
  dragging,
  onDragStart,
  onDragEnd,
  filing,
  onFile,
  destinations,
  onMoveTo,
  onUnnest,
  children,
  expanded,
  onToggle,
  onAdd,
  dropping = false,
  dropHint,
  ...drag
}: {
  page: Page;
  depth: number;
  selected: boolean;
  onOpen: () => void;
  dragging: boolean;
  onDragStart: () => void;
  onDragEnd: () => void;
  /** Whether this row's list of destinations is showing. */
  filing: boolean;
  onFile: () => void;
  destinations: string[];
  onMoveTo: (folder: string) => void;
  /** Take this page back out of the one it is in. Absent when it is not in one. */
  onUnnest?: () => void;
  /** How many pages are inside this one. */
  children: number;
  expanded: boolean;
  onToggle: () => void;
  /** Make a page inside this one. */
  onAdd: () => void;
  dropping?: boolean;
  dropHint?: string;
}) {
  const t = useT();
  const Glyph = page.kind === "meeting" ? Mic : FileText;
  const elsewhere = destinations.filter((folder) => folder !== (page.folder ?? ""));

  return (
    <div className={cn("group/page", dragging && "opacity-40")}>
      <div
        {...drag}
        title={dropHint}
        // The row is what gets dragged, not the button inside it: a `draggable` button in Chromium
        // starts a drag on `mousedown` and then never fires `click`, so making the page itself
        // draggable would have cost the ability to open it.
        draggable
        onDragStart={(event) => {
          // Some payload is required — a drag with an empty `dataTransfer` is refused outright by
          // Firefox. What it *says* is unused: the dragged page is held in state, because the data
          // is deliberately unreadable until the drop.
          event.dataTransfer.setData("text/plain", page.id);
          event.dataTransfer.effectAllowed = "move";
          onDragStart();
        }}
        onDragEnd={onDragEnd}
        style={{ paddingLeft: `${depth * 12}px` }}
        className={cn(
          "flex w-full items-center gap-1 rounded-lg py-1 pe-1 text-sm transition-colors",
          // Two different facts are marked in this column and they must not look alike: which
          // *folder* is being browsed, and which *page* is open. The folder takes the neutral step
          // up; the open page takes the accent, the same way the notes list and the screen nav mark
          // the thing you are currently looking at.
          //
          // Both were `bg-soft` — and the sidebar's own surface *is* `bg-soft`, so neither the
          // hover nor the selection painted anything at all. Nothing catches that but looking: the
          // class is present, it resolves, and the computed style is exactly what was asked for.
          selected ? "bg-accent-soft text-accent font-medium" : "text-fg-dim hover:bg-bg-raised",
          dropping && "outline-accent bg-accent-soft outline-1",
        )}
      >
        {/* The chevron occupies its space whether or not there is anything to expand, so a column
            of titles stays a column rather than stepping in and out by six pixels. */}
        {children > 0 ? (
          <button
            type="button"
            onClick={onToggle}
            aria-expanded={expanded}
            aria-label={t(expanded ? "nav.collapse" : "nav.expand", { name: page.title })}
            className="text-fg-faint hover:text-fg flex size-4 shrink-0 items-center justify-center"
          >
            <ChevronRight
              aria-hidden="true"
              className={cn("size-3 transition-transform duration-150", expanded && "rotate-90")}
            />
          </button>
        ) : (
          <span className="size-4 shrink-0" aria-hidden="true" />
        )}
        {/* The icon is the only thing that says which kind this is, and that is enough: the
            difference matters when you are looking for a recording, and never otherwise. */}
        <Glyph className="text-fg-faint size-3.5 shrink-0" aria-hidden="true" />
        <button type="button" onClick={onOpen} className="min-w-0 flex-1 truncate text-start">
          {page.title}
        </button>
        {/* A page inside this one, from the row it will be inside. This is how nearly every
            sub-page gets made — the slash menu in the editor is for the one you think of while
            writing, and this is for the one you think of while looking at the tree. */}
        <button
          type="button"
          onClick={onAdd}
          aria-label={t("nav.new_page_in", { name: page.title })}
          className={cn(
            "text-fg-faint hover:text-fg flex size-5 shrink-0 items-center justify-center rounded transition-opacity",
            "opacity-0 group-hover/page:opacity-100 focus-visible:opacity-100",
          )}
        >
          <Plus className="size-3.5" aria-hidden="true" />
        </button>
        {/* Dragging is a pointer gesture and this app runs on phones, where HTML5 drag events are
            not delivered at all. So this is not a fallback for the drag — it is the only way to
            file anything on touch, and the only way with a keyboard. */}
        {(elsewhere.length > 0 || onUnnest) && (
          <button
            type="button"
            onClick={onFile}
            aria-expanded={filing}
            aria-label={t("nav.move_page", { name: page.title })}
            className={cn(
              "text-fg-faint hover:text-fg flex size-5 shrink-0 items-center justify-center rounded transition-opacity",
              // Shown on hover and whenever it has focus, so tabbing to it does not tab to
              // something invisible. `opacity` rather than `hidden`, because a row that changes
              // width on hover makes the title reflow under the pointer.
              filing
                ? "text-fg opacity-100"
                : "opacity-0 group-hover/page:opacity-100 focus-visible:opacity-100",
            )}
          >
            <FolderInput className="size-3.5" aria-hidden="true" />
          </button>
        )}
      </div>

      {/* In the flow rather than floating over it. The tree scrolls in both directions, so a
          popup anchored to a row is a popup that gets clipped by the column it lives in. */}
      {filing && (
        <ul
          data-testid="move-page"
          aria-label={t("nav.move_page", { name: page.title })}
          className="border-line bg-bg-raised my-0.5 ms-6 me-1 rounded-lg border py-1"
        >
          {/* First, because it is the one destination that is not a place: it undoes the nesting
              rather than choosing a different one. Only offered when there is something to undo. */}
          {onUnnest && (
            <li>
              <button
                type="button"
                onClick={onUnnest}
                className="text-fg-dim hover:bg-bg-raised hover:text-fg text-meta flex w-full items-center gap-1.5 px-2 py-1 text-start"
              >
                <FileText className="text-fg-faint size-3 shrink-0" aria-hidden="true" />
                <span className="truncate">{t("nav.unnest")}</span>
              </button>
            </li>
          )}
          {elsewhere.map((folder) => (
            <li key={folder || "/"}>
              <button
                type="button"
                onClick={() => onMoveTo(folder)}
                className="text-fg-dim hover:bg-bg-raised hover:text-fg text-meta flex w-full items-center gap-1.5 px-2 py-1 text-start"
              >
                <Folder className="text-fg-faint size-3 shrink-0" aria-hidden="true" />
                <span className="truncate">{folder || t("nav.move_to_root")}</span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Pages at the top of a folder: filed there, and inside no other page. */
function topLevel(inside: Map<string, Page[]>, folder: string): Page[] {
  return (inside.get("") ?? []).filter((page) => (page.folder ?? "") === folder);
}

/** Every page inside this one, however deep. Used to refuse a nesting that would make a loop. */
function descendants(inside: Map<string, Page[]>, id: string): Set<string> {
  const found = new Set<string>();
  const queue = [id];
  while (queue.length > 0) {
    for (const child of inside.get(queue.pop()!) ?? []) {
      if (found.has(child.id)) continue;
      found.add(child.id);
      queue.push(child.id);
    }
  }
  return found;
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
  dropping = false,
  dropHint,
  ...drag
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
  /** Whether a page is being dragged over this row right now. */
  dropping?: boolean;
  /** What dropping here would do, for the pointer's tooltip. */
  dropHint?: string;
  onDragOver?: (event: React.DragEvent) => void;
  onDragEnter?: () => void;
  onDragLeave?: () => void;
  onDrop?: (event: React.DragEvent) => void;
}) {
  const t = useT();
  return (
    <div
      {...drag}
      title={dropHint}
      className={cn(
        "flex items-center rounded-lg text-sm transition-colors",
        // `raised` for the same reason as the page rows: the sidebar is `bg-soft`, so a highlight
        // of `bg-soft` is no highlight.
        selected ? "bg-bg-raised text-fg font-medium" : "text-fg-dim hover:bg-bg-raised",
        // An outline rather than a background: the selected folder already owns the background, and
        // a drop target that only changed colour was indistinguishable from the row you came from.
        dropping && "outline-accent bg-accent-soft outline-1",
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
        // `isolate` is load-bearing. The highlight below is `-z-10` so the label paints over it,
        // but without a stacking context here that negative index escapes the button entirely and
        // the pill painted *behind the sidebar's own background* — invisible on every screen. The
        // selected row had been nothing but green text for as long as this component has existed.
        "relative isolate flex w-full items-center gap-2.5 rounded-[var(--radius-pill)] px-2.5 py-2 text-sm transition-colors",
        active ? "text-accent font-medium" : "text-fg-dim hover:bg-bg-raised hover:text-fg",
      )}
    >
      {/* The highlight is one element that moves between rows rather than a background that blinks
          on and off. `layoutId` makes the browser interpolate it, so changing screen reads as the
          selection travelling down the list — which is the thing that tells you the list is one
          list. */}
      {active && (
        <m.span
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
        <span className="nums bg-rec text-micro ml-auto rounded-full px-1.5 py-0.5 font-semibold text-white">
          {item.badge}
        </span>
      ) : null}
    </button>
  );
}
