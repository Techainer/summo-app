import { useMemo, useState } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { ancestorsOf, buildTree, visibleRows } from "../../lib/folders";

export interface NavItem {
  key: string;
  label: string;
  icon: string;
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
        <p className="px-2 pb-1.5 text-[11px] font-semibold uppercase tracking-wider text-fg-faint">
          {t("nav.menu")}
        </p>
        <ul className="space-y-0.5">
          {items.map((item) => (
            <li key={item.key}>
              <button
                type="button"
                onClick={() => onNavigate(item.key)}
                aria-current={active === item.key ? "page" : undefined}
                className={cn(
                  "flex w-full items-center gap-2.5 rounded-lg px-2 py-2 text-sm transition-colors",
                  active === item.key
                    ? "bg-accent-soft font-medium text-accent"
                    : "text-fg-dim hover:bg-bg-soft hover:text-fg",
                )}
              >
                <span aria-hidden="true" className="w-4 text-center">
                  {item.icon}
                </span>
                {item.label}
                {item.badge ? (
                  <span className="tabular ml-auto rounded-full bg-rec px-1.5 py-0.5 text-[11px] font-semibold text-white">
                    {item.badge}
                  </span>
                ) : null}
              </button>
            </li>
          ))}
        </ul>
      </nav>

      <div className="mt-4 border-t border-line" />

      <nav aria-label={t("nav.folders")} className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <p className="px-2 pb-1.5 text-[11px] font-semibold uppercase tracking-wider text-fg-faint">
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
          {rows.map((node) => (
            <li key={node.path}>
              <FolderRow
                label={node.name}
                depth={node.depth}
                selected={activeFolder === node.path}
                expandable={node.children.length > 0}
                expanded={open.has(node.path)}
                onToggle={() => toggle(node.path)}
                onSelect={() => onSelectFolder(node.path)}
              />
            </li>
          ))}
          {rows.length === 0 && (
            <li className="px-2 py-1 text-[13px] text-fg-faint">{t("nav.no_folders")}</li>
          )}
        </ul>
      </nav>

      {footer && <div className="border-t border-line p-3">{footer}</div>}
    </div>
  );
}

function FolderRow({
  label,
  depth,
  selected,
  expandable = false,
  expanded = false,
  onToggle,
  onSelect,
}: {
  label: string;
  depth: number;
  selected: boolean;
  expandable?: boolean;
  expanded?: boolean;
  onToggle?: () => void;
  onSelect: () => void;
}) {
  const t = useT();
  return (
    <div
      className={cn(
        "flex items-center rounded-lg text-sm transition-colors",
        selected ? "bg-bg-soft font-medium text-fg" : "text-fg-dim hover:bg-bg-soft",
      )}
      // Indent by nesting level. Padding rather than margin so the hover target stays full width.
      style={{ paddingLeft: `${depth * 12}px` }}
    >
      {expandable ? (
        <button
          type="button"
          onClick={onToggle}
          aria-label={t(expanded ? "nav.collapse" : "nav.expand", { name: label })}
          aria-expanded={expanded}
          className="flex size-6 shrink-0 items-center justify-center text-fg-faint hover:text-fg"
        >
          <span
            aria-hidden="true"
            className={cn("transition-transform duration-150", expanded && "rotate-90")}
          >
            ›
          </span>
        </button>
      ) : (
        <span className="size-6 shrink-0" aria-hidden="true" />
      )}
      <button
        type="button"
        onClick={onSelect}
        className="flex-1 truncate py-1.5 pr-2 text-left"
        title={label}
      >
        {label}
      </button>
    </div>
  );
}
