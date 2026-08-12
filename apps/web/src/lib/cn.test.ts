import { describe, expect, it } from "vitest";

import { cn } from "./cn";

describe("cn", () => {
  it("keeps a colour when a size from our own scale follows it", () => {
    // The bug this guards: `tailwind-merge` reads `text-meta` as a colour unless told otherwise, so
    // it dropped `text-accent-fg` and every primary button rendered its label in the page's default
    // foreground — 1.39:1 against the green fill.
    expect(cn("text-accent-fg", "text-meta")).toBe("text-accent-fg text-meta");
  });

  it("still lets one size replace another", () => {
    expect(cn("text-body", "text-micro")).toBe("text-micro");
  });

  it("still lets one colour replace another", () => {
    expect(cn("text-fg-dim", "text-accent")).toBe("text-accent");
  });
});
