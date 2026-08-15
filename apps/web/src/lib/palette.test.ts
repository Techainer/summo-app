import { describe, expect, it } from "vitest";

import {
  NAVIGATION_LENGTH,
  asThings,
  fold,
  matchCommands,
  order,
  type Action,
  type Place,
} from "./palette";
import type { SearchHit } from "./library";

const places: Place[] = [
  { kind: "place", to: "/models", label: "Mô hình", keywords: ["models", "pull"] },
  { kind: "place", to: "/tasks", label: "Việc", keywords: ["tasks", "todo"] },
  { kind: "place", to: "/library", label: "Kho", keywords: ["vault", "library"] },
];

describe("folding", () => {
  // A Vietnamese speaker searching their own notes on a keyboard with no Vietnamese layout is the
  // normal case. A palette that needs the tone marks typed is a palette abandoned on first use.
  it("ignores tone marks and case", () => {
    expect(fold("Mô hình")).toBe("mo hinh");
    expect(fold("Việc cần làm")).toBe("viec can lam");
    expect(fold("ĐÃ XONG")).toBe("da xong");
  });
});

describe("places", () => {
  it("finds a screen by its label without diacritics", () => {
    expect(matchCommands(places, "mo hinh").map((p) => p.to)).toEqual(["/models"]);
  });

  // The interface language is not the language somebody's fingers default to. `models` has to find
  // `Mô hình` even when the app is in Vietnamese.
  it("finds a screen by an English keyword in a Vietnamese interface", () => {
    expect(matchCommands(places, "models").map((p) => p.to)).toEqual(["/models"]);
  });

  it("requires every word, so a second word narrows", () => {
    expect(matchCommands(places, "kho vault").map((p) => p.to)).toEqual(["/library"]);
    expect(matchCommands(places, "kho models")).toEqual([]);
  });

  it("offers everything for an empty query", () => {
    expect(matchCommands(places, "  ")).toHaveLength(places.length);
  });
});

describe("ordering", () => {
  const things = asThings([
    {
      meeting: { id: "01A", kind: "meeting", title: "Họp đầu tuần", day: "2026-08-10" },
      matches: 1,
      excerpts: [{ text: "  ngân sách  " }],
    } as unknown as SearchHit,
  ]);

  // Two letters is somebody navigating; a sentence is somebody searching. Guessing wrong puts the
  // wrong half of the list under the cursor at the moment they press Enter.
  it("puts screens first while the query is short", () => {
    expect(order(places, things, "mo")[0]?.kind).toBe("place");
  });

  it("puts vault hits first once it is a search", () => {
    expect("ngan sach".length).toBeGreaterThanOrEqual(NAVIGATION_LENGTH);
    expect(order(places, things, "ngan sach")[0]?.kind).toBe("thing");
  });

  it("keeps both halves either way", () => {
    expect(order(places, things, "mo")).toHaveLength(places.length + things.length);
    expect(order(places, things, "ngan sach")).toHaveLength(places.length + things.length);
  });
});

describe("vault hits", () => {
  it("carries the kind, because it decides where opening it goes", () => {
    const [thing] = asThings([
      {
        meeting: { id: "01N", kind: "note", title: "Ý tưởng", day: "2026-08-12" },
        matches: 1,
        excerpts: [],
      } as unknown as SearchHit,
    ]);
    expect(thing?.entry).toBe("note");
    expect(thing?.excerpt).toBeUndefined();
  });

  it("trims the excerpt it shows", () => {
    const [thing] = asThings([
      {
        meeting: { id: "01A", kind: "meeting", title: "Họp", day: "2026-08-10" },
        matches: 1,
        excerpts: [{ text: "  chốt spec  " }],
      } as unknown as SearchHit,
    ]);
    expect(thing?.excerpt).toBe("chốt spec");
  });
});

describe("actions", () => {
  const doing: Action[] = [
    { kind: "action", id: "record", label: "Ghi", keywords: ["record", "thu"], run: () => {} },
  ];
  const going: Place[] = [{ kind: "place", to: "/record", label: "Ghi", keywords: ["record"] }];

  /**
   * A verb is an instruction. Somebody who typed "ghi" has said what they want to happen, and
   * making them walk past two screens with that word in the name to reach it is the palette
   * failing at the one thing it is for.
   */
  it("puts what you can do above where you can go", () => {
    const shown = order([...going, ...doing], [], "ghi");
    expect(shown[0]).toMatchObject({ kind: "action", id: "record" });
  });

  it("keeps them first even when the query is long enough to be a search", () => {
    const found = asThings([
      {
        meeting: { id: "01A", title: "Ghi chép", day: "2026-08-10", kind: "note" },
        matches: 1,
        excerpts: [],
      },
    ] as never);
    const shown = order(doing, found, "ghi chép hôm nay");
    expect(shown[0]).toMatchObject({ kind: "action" });
    expect(shown[1]).toMatchObject({ kind: "thing" });
  });

  it("finds one by a keyword in another language, like everything else here", () => {
    expect(matchCommands(doing, "record")).toHaveLength(1);
    expect(matchCommands(doing, "khong co")).toHaveLength(0);
  });
});
