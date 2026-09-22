import { describe, expect, it } from "vitest";

import { accuracyFor, type CatalogueModel } from "./catalogue";

/** A card with only the fields this rule reads. */
function model(langs: string[], accuracy: { lang: string; accuracy: number }[]): CatalogueModel {
  return {
    id: "x",
    name: "X",
    task: "asr",
    mode: "live",
    langs,
    license: "MIT",
    redistributable: true,
    gated: false,
    size_bytes: 1,
    installed: false,
    fits: true,
    min_ram_mb: 0,
    accuracy,
  };
}

describe("which measurement a card shows", () => {
  it("prefers the reader's own language", () => {
    const m = model(
      ["*"],
      [
        { lang: "en", accuracy: 0.9 },
        { lang: "vi", accuracy: 0.6 },
      ],
    );
    expect(accuracyFor(m, "vi-VN")?.lang).toBe("vi");
  });

  /**
   * The case this rule was rewritten for.
   *
   * `zipformer-en` was measured at 59.7 % WER — 40 % accurate — and rendered as "chưa đo" on a
   * Vietnamese interface, because the card looked for a Vietnamese figure on an English-only model
   * and found none. A model measured and found bad displayed exactly like a model nobody had
   * tested, on the screen whose whole job is to warn about that.
   */
  it("shows an English-only model's English figure to a Vietnamese reader", () => {
    const m = model(["en"], [{ lang: "en", accuracy: 0.403 }]);
    const shown = accuracyFor(m, "vi-VN");
    expect(shown?.lang).toBe("en");
    expect(shown?.accuracy).toBeCloseTo(0.403);
  });

  /**
   * And the other half, which keeps it honest: a model that *does* serve the reader's language and
   * has no figure for it is unmeasured **for them**. Showing its score in another language would
   * answer a question they did not ask — a multilingual model good at English says nothing about
   * what it will do to their Vietnamese.
   */
  it("says nothing when the model covers the reader's language but was not measured on it", () => {
    const m = model(["*"], [{ lang: "en", accuracy: 0.9 }]);
    expect(accuracyFor(m, "vi-VN")).toBeUndefined();
  });

  it("has nothing to show for a model nobody measured", () => {
    expect(accuracyFor(model(["en"], []), "vi-VN")).toBeUndefined();
  });
});
