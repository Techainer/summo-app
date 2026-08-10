import { describe, expect, it } from "vitest";

import { ancestorsOf, buildTree, visibleRows, type FolderNode } from "./folders";

/** Index into a node list, failing the test rather than returning undefined. */
function at(nodes: FolderNode[], index: number): FolderNode {
  const node = nodes[index];
  if (!node) throw new Error(`no node at ${index} in [${nodes.map((n) => n.path).join(", ")}]`);
  return node;
}

describe("buildTree", () => {
  it("returns nothing for an empty vault", () => {
    expect(buildTree([])).toEqual([]);
  });

  it("keeps top-level folders flat", () => {
    const tree = buildTree(["Khách hàng", "Sản phẩm"]);
    expect(tree.map((n) => n.name)).toEqual(["Khách hàng", "Sản phẩm"]);
    expect(tree.every((n) => n.depth === 0)).toBe(true);
  });

  it("nests a child under its parent", () => {
    const tree = buildTree(["Sản phẩm", "Sản phẩm/Weekly"]);
    expect(tree).toHaveLength(1);
    expect(at(tree, 0).children.map((n) => n.name)).toEqual(["Weekly"]);
    expect(at(at(tree, 0).children, 0).path).toBe("Sản phẩm/Weekly");
    expect(at(at(tree, 0).children, 0).depth).toBe(1);
  });

  /// The daemon reports only folders that hold meetings, so intermediate ones can be missing.
  it("creates an ancestor that was never reported", () => {
    const tree = buildTree(["Sản phẩm/Weekly/2026"]);
    expect(tree.map((n) => n.name)).toEqual(["Sản phẩm"]);
    expect(at(at(tree, 0).children, 0).name).toBe("Weekly");
    expect(at(at(at(tree, 0).children, 0).children, 0).name).toBe("2026");
  });

  it("sorts by Vietnamese collation, not code points", () => {
    const tree = buildTree(["Zung", "Ánh", "An"]);
    expect(tree.map((n) => n.name)).toEqual(["An", "Ánh", "Zung"]);
  });

  it("sorts children too", () => {
    const tree = buildTree(["A/Zung", "A/Ánh"]);
    expect(at(tree, 0).children.map((n) => n.name)).toEqual(["Ánh", "Zung"]);
  });

  it("does not duplicate a folder reported twice", () => {
    const tree = buildTree(["Sản phẩm", "Sản phẩm"]);
    expect(tree).toHaveLength(1);
  });

  it("ignores empty segments from a stray slash", () => {
    const tree = buildTree(["Sản phẩm//Weekly", ""]);
    expect(tree).toHaveLength(1);
    expect(at(tree, 0).children.map((n) => n.name)).toEqual(["Weekly"]);
  });

  it("handles deep nesting without losing the path", () => {
    const tree = buildTree(["a/b/c/d"]);
    let node = at(tree, 0);
    while (node.children.length > 0) node = at(node.children, 0);
    expect(node.path).toBe("a/b/c/d");
    expect(node.depth).toBe(3);
  });
});

describe("ancestorsOf", () => {
  it("lists every level down to the folder", () => {
    expect(ancestorsOf("a/b/c")).toEqual(["a", "a/b", "a/b/c"]);
  });

  it("is empty for the vault root", () => {
    expect(ancestorsOf("")).toEqual([]);
  });
});

describe("visibleRows", () => {
  const tree = buildTree(["A", "A/B", "A/B/C", "D"]);

  it("shows only roots when nothing is open", () => {
    expect(visibleRows(tree, new Set()).map((n) => n.path)).toEqual(["A", "D"]);
  });

  it("reveals children of an open folder", () => {
    expect(visibleRows(tree, new Set(["A"])).map((n) => n.path)).toEqual(["A", "A/B", "D"]);
  });

  it("does not reveal a grandchild when the middle folder is shut", () => {
    // Opening A and A/B/C without A/B must not leak C onto the screen.
    expect(visibleRows(tree, new Set(["A", "A/B/C"])).map((n) => n.path)).toEqual([
      "A",
      "A/B",
      "D",
    ]);
  });

  it("reveals the whole chain when every ancestor is open", () => {
    const open = new Set(ancestorsOf("A/B/C"));
    expect(visibleRows(tree, open).map((n) => n.path)).toEqual(["A", "A/B", "A/B/C", "D"]);
  });
});
