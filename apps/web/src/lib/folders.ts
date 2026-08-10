/**
 * Turning the vault's flat folder list into a tree.
 *
 * The daemon reports folders as paths — `Sản phẩm`, `Sản phẩm/Weekly`, `Khách hàng` — because that
 * is what the filesystem is. The sidebar in the Coreto reference is a tree, so the conversion has
 * to happen somewhere, and doing it here keeps it testable without a DOM.
 *
 * Two rules that are easy to get wrong:
 *
 * * A folder that exists only as an ancestor (`Sản phẩm` when only `Sản phẩm/Weekly` was reported)
 *   still needs a node, or its children have nowhere to hang.
 * * Sorting is by Vietnamese collation, not code points, so `Ánh` lands next to `An` rather than
 *   after `Zung`.
 */

export interface FolderNode {
  /** Full path, which is what the API takes: `Sản phẩm/Weekly`. */
  path: string;
  /** Last segment, which is what the user reads: `Weekly`. */
  name: string;
  depth: number;
  children: FolderNode[];
}

export function buildTree(folders: string[]): FolderNode[] {
  const roots: FolderNode[] = [];
  const byPath = new Map<string, FolderNode>();

  // Shortest first, so a parent is always created before the child that needs it.
  const paths = [...new Set(folders.filter(Boolean))].sort(
    (a, b) => a.split("/").length - b.split("/").length,
  );

  for (const path of paths) {
    const segments = path.split("/").filter(Boolean);
    let prefix = "";
    let siblings = roots;
    let depth = 0;

    for (const segment of segments) {
      prefix = prefix ? `${prefix}/${segment}` : segment;
      let node = byPath.get(prefix);
      if (!node) {
        // Creates the implied ancestor as well as the folder that was actually reported.
        node = { path: prefix, name: segment, depth, children: [] };
        byPath.set(prefix, node);
        siblings.push(node);
      }
      siblings = node.children;
      depth += 1;
    }
  }

  sortTree(roots);
  return roots;
}

function sortTree(nodes: FolderNode[]) {
  nodes.sort((a, b) => a.name.localeCompare(b.name, "vi"));
  for (const node of nodes) sortTree(node.children);
}

/** Every path from the root down to `path`, so the tree can open to reveal a selection. */
export function ancestorsOf(path: string): string[] {
  const segments = path.split("/").filter(Boolean);
  const out: string[] = [];
  let prefix = "";
  for (const segment of segments) {
    prefix = prefix ? `${prefix}/${segment}` : segment;
    out.push(prefix);
  }
  return out;
}

/** Flatten to the rows actually on screen, given which folders are open. */
export function visibleRows(nodes: FolderNode[], open: ReadonlySet<string>): FolderNode[] {
  const out: FolderNode[] = [];
  const walk = (list: FolderNode[]) => {
    for (const node of list) {
      out.push(node);
      if (node.children.length > 0 && open.has(node.path)) walk(node.children);
    }
  };
  walk(nodes);
  return out;
}
