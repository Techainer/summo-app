import { FolderOpen, Inbox } from "lucide-react";
import { useMemo } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { swatch, type LibraryView } from "../../lib/library";

/**
 * Where a vault gets organised: folders, tags, colours.
 *
 * Three axes rather than one because they answer different questions, and a user reaches for
 * whichever they happen to remember. A folder is where something *lives* — one per note, exclusive,
 * the thing a file manager shows. A tag is what it is *about* — many per note, and the same note is
 * legitimately both `khách-hàng` and `hợp-đồng`. A colour is neither; it is a mark somebody put on a
 * note so they could find it again, and it means whatever they decided it means.
 *
 * All three live in the files (ADR 0006). Nothing here is a query against an index: the daemon
 * rescans, which is why a tag typed into a `.md` in Obsidian shows up in this panel with no import.
 *
 * ## The selection model
 *
 * Tags are additive and everything else is exclusive, which matches what each is for. Picking two
 * tags means *both* — one tag is a hundred notes and two are the four you wanted, so an `or` there
 * would make the second click widen the search, which is the opposite of what clicking a filter is
 * for. Two folders or two colours would have to be an `or`, and neither is a question people ask.
 */
interface Props {
  view: LibraryView;
  folder: string | undefined;
  tags: string[];
  colour: string | undefined;
  onFolder: (folder: string | undefined) => void;
  onTags: (tags: string[]) => void;
  onColour: (colour: string | undefined) => void;
}

export function Finder({ view, folder, tags, colour, onFolder, onTags, onColour }: Props) {
  const t = useT();
  const tree = useMemo(() => toTree(view.folders), [view.folders]);
  const active = folder !== undefined || tags.length > 0 || colour !== undefined;

  return (
    <div className="flex flex-col gap-3" data-testid="finder">
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="text-fg-faint text-micro font-semibold tracking-wide uppercase">
          {t("finder.title")}
        </h2>
        {/* One control that undoes every filter at once. Clearing them one at a time is three
            clicks and a memory test — the user has to recall what they narrowed by. */}
        {active && (
          <button
            type="button"
            className="text-fg-dim hover:bg-bg-soft hover:text-fg text-micro rounded-md px-1.5 py-0.5"
            onClick={() => {
              onFolder(undefined);
              onTags([]);
              onColour(undefined);
            }}
          >
            {t("finder.clear")}
          </button>
        )}
      </div>

      {tree.length > 1 && (
        <nav aria-label={t("library.filter_folder")} className="flex flex-col">
          {tree.map((node) => (
            <button
              key={node.path}
              type="button"
              // Clicking the folder you are already in leaves it, so the same target both enters
              // and exits. A separate "all folders" row would be a second thing to aim at.
              onClick={() => onFolder(folder === node.path ? undefined : node.path)}
              aria-pressed={folder === node.path}
              style={{ paddingInlineStart: `${8 + node.depth * 14}px` }}
              className={cn(
                "text-meta flex items-center gap-1.5 rounded-md py-1 pe-2 text-left transition-colors",
                folder === node.path
                  ? "bg-bg-soft text-fg font-medium"
                  : "text-fg-dim hover:bg-bg-soft hover:text-fg",
              )}
            >
              {/* `Inbox` for the root rather than a folder: what sits there is not filed, and a
                  folder icon on "unfiled" says the opposite of what the row means. */}
              {node.path === "" ? (
                <Inbox
                  aria-hidden="true"
                  className="text-fg-faint size-3.5 shrink-0 stroke-[1.75]"
                />
              ) : (
                <FolderOpen
                  aria-hidden="true"
                  className="text-fg-faint size-3.5 shrink-0 stroke-[1.75]"
                />
              )}
              <span className="truncate">
                {node.path === "" ? t("library.unfiled") : node.name}
              </span>
            </button>
          ))}
        </nav>
      )}

      {view.tags.length > 0 && (
        <nav aria-label={t("library.filter_tag")} className="flex flex-wrap gap-1.5">
          {view.tags.map((each) => {
            const on = tags.includes(each.name);
            return (
              <button
                key={each.name}
                type="button"
                aria-pressed={on}
                onClick={() =>
                  onTags(on ? tags.filter((x) => x !== each.name) : [...tags, each.name])
                }
                className={cn(
                  "text-micro rounded-full border px-2.5 py-0.5 transition-colors",
                  on
                    ? "border-accent bg-accent-soft text-accent"
                    : "border-line text-fg-dim hover:border-line-strong hover:text-fg",
                )}
              >
                #{each.name}
                <span className="nums text-fg-faint ms-1">{each.count}</span>
              </button>
            );
          })}
        </nav>
      )}

      {view.colours.length > 0 && (
        <nav aria-label={t("library.filter_colour")} className="flex flex-wrap gap-1.5">
          {view.colours.map((each) => {
            const on = colour === each.name;
            return (
              <button
                key={each.name}
                type="button"
                aria-pressed={on}
                onClick={() => onColour(on ? undefined : each.name)}
                className={cn(
                  "text-micro flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 transition-colors",
                  on
                    ? "border-fg-faint bg-bg-soft text-fg"
                    : "border-line text-fg-dim hover:border-line-strong hover:text-fg",
                )}
              >
                <Dot colour={each.name} />
                {/* The name in words, not only the dot. A row of chips reading "1 1 1 1 1" is what
                    this looked like when the dot was the whole label — and that is also what it
                    degrades to for anyone who cannot tell two of these colours apart, which is
                    around one man in twelve. */}
                {t(`colour.${each.name}`)}
                <span className="nums text-fg-faint">{each.count}</span>
              </button>
            );
          })}
        </nav>
      )}
    </div>
  );
}

/**
 * The mark itself.
 *
 * A ring rather than a filled circle at 10 px: a solid dot of `swatch-grey` on `bg-soft` is two
 * greys a few percent apart, and the ring gives it an edge to be seen by in either scheme.
 */
export function Dot({ colour, className }: { colour: string; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block size-2.5 shrink-0 rounded-full ring-1 ring-black/20 ring-inset",
        className,
      )}
      style={{ background: swatch(colour) }}
    />
  );
}

/**
 * The eight swatches, on one note.
 *
 * A row of radio buttons rather than a dropdown: eight options that are *only* distinguishable by
 * appearance are the case a `<select>` is worst at, since its list shows names. It is a real radio
 * group so arrow keys move between the colours and a screen reader announces "3 of 9" — and so
 * that clearing is a ninth option in the same group rather than a separate control the keyboard
 * has to travel to.
 */
export function ColourPicker({
  palette,
  chosen,
  disabled,
  onChoose,
}: {
  palette: string[];
  chosen: string | null;
  disabled?: boolean;
  onChoose: (colour: string | null) => void;
}) {
  const t = useT();
  return (
    // `shrink-0`: the swatches are fixed-size, so in a flex row that is short of space this box
    // gets squeezed below its contents and the dots spill over whatever is beside them rather than
    // wrapping. Refusing to shrink is what makes the row wrap instead.
    <div
      role="radiogroup"
      aria-label={t("finder.colour")}
      className="flex shrink-0 flex-wrap items-center gap-1"
    >
      <button
        type="button"
        role="radio"
        aria-checked={chosen === null}
        aria-label={t("finder.no_colour")}
        disabled={disabled}
        onClick={() => onChoose(null)}
        className={cn(
          "text-fg-faint text-micro grid size-6 place-items-center rounded-full border transition-colors disabled:opacity-50",
          chosen === null ? "border-fg-faint bg-bg-soft" : "border-line hover:border-line-strong",
        )}
      >
        <span aria-hidden="true">✕</span>
      </button>
      {palette.map((name) => (
        <button
          key={name}
          type="button"
          role="radio"
          aria-checked={chosen === name}
          aria-label={t(`colour.${name}`)}
          disabled={disabled}
          onClick={() => onChoose(name)}
          className={cn(
            "grid size-6 place-items-center rounded-full border transition-colors disabled:opacity-50",
            chosen === name
              ? "border-fg-faint bg-bg-soft"
              : "hover:border-line-strong border-transparent",
          )}
        >
          <Dot colour={name} className="size-3.5" />
        </button>
      ))}
    </div>
  );
}

interface Node {
  /** Full path, `""` for the vault root. */
  path: string;
  /** Last segment, which is what a tree row shows. */
  name: string;
  depth: number;
}

/**
 * Flat folder paths as an indented tree.
 *
 * The daemon sends `["", "khach-hang", "khach-hang/acme"]`, already sorted, which is exactly
 * depth-first order — so this is indentation, not a graph. Keeping it that way means a folder
 * nobody has filed anything into simply is not there, which is the truth: folders exist because
 * files are in them.
 */
function toTree(folders: string[]): Node[] {
  return folders.map((path) => ({
    path,
    name: path.slice(path.lastIndexOf("/") + 1),
    depth: path === "" ? 0 : path.split("/").length - 1,
  }));
}
