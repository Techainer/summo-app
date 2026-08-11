import { describe, expect, it, vi as vitest } from "vitest";

import {
  BUILT_IN,
  DEFAULT,
  catalogFor,
  detectLocale,
  flatten,
  interpolate,
  mergeCatalogs,
  plural,
  translator,
} from ".";
import en from "./en.json";
import viJson from "./vi.json";

describe("flatten", () => {
  it("turns nested json into dotted keys", () => {
    expect(flatten({ nav: { record: "Ghi" } })).toEqual({
      "nav.record": "Ghi",
    });
  });

  it("drops values that are not strings rather than coercing them", () => {
    expect(flatten({ a: 1, b: true, c: "x" })).toEqual({ c: "x" });
  });

  it("survives a file that is not an object at all", () => {
    expect(flatten("nope")).toEqual({});
    expect(flatten(null)).toEqual({});
  });
});

describe("translator", () => {
  const t = translator("vi", { greet: "Chào {name}" });

  it("substitutes placeholders", () => {
    expect(t.t("greet", { name: "Ngọc" })).toBe("Chào Ngọc");
  });

  // A blank label is invisible: it ships, nobody notices, and a user finds an unlabelled button.
  it("renders the key when it is missing, never blank", () => {
    expect(t.t("nope.missing")).toBe("nope.missing");
  });

  it("leaves a placeholder alone when no value was passed", () => {
    expect(t.t("greet")).toBe("Chào {name}");
  });

  it("reports each missing key once, not once per render", () => {
    const missing = vitest.fn();
    const reporting = translator("vi", {}, missing);
    reporting.t("a");
    reporting.t("a");
    reporting.t("b");
    expect(missing).toHaveBeenCalledTimes(2);
  });

  it("knows whether a key exists", () => {
    expect(t.has("greet")).toBe(true);
    expect(t.has("greet.nope")).toBe(false);
  });
});

describe("plurals", () => {
  const catalog = {
    "task.open_one": "{count} task open",
    "task.open_other": "{count} tasks open",
  };

  it("picks the English singular and plural", () => {
    const t = translator("en", catalog);
    expect(t.n("task.open", 1)).toBe("1 task open");
    expect(t.n("task.open", 3)).toBe("3 tasks open");
  });

  // Vietnamese has no plural agreement; Intl reports "other" for every count, and both forms in
  // vi.json are the same string. This is the case a hand-rolled `count === 1` gets wrong.
  it("uses one form for Vietnamese whatever the count", () => {
    const t = translator("vi", {
      "task.open_one": "{count} việc",
      "task.open_other": "{count} việc",
    });
    expect(t.n("task.open", 1)).toBe("1 việc");
    expect(t.n("task.open", 5)).toBe("5 việc");
  });

  it("falls back to the other form when a language lacks a category", () => {
    expect(plural({ a_other: "many" }, "a", 1, "en")).toBe("many");
  });

  it("does not throw on a locale tag Intl does not know", () => {
    expect(plural({ a_other: "x" }, "a", 2, "not-a-locale!!")).toBe("x");
  });

  it("renders the key rather than blank when no form exists", () => {
    expect(translator("en", {}).n("gone", 2)).toBe("gone");
  });
});

describe("interpolate", () => {
  it("replaces every occurrence", () => {
    expect(interpolate("{a} and {a}", { a: "x" })).toBe("x and x");
  });

  it("accepts numbers", () => {
    expect(interpolate("{n}%", { n: 42 })).toBe("42%");
  });
});

describe("fallbacks", () => {
  it("layers later catalogs over earlier ones", () => {
    expect(mergeCatalogs({ a: "1", b: "2" }, { b: "3" })).toEqual({
      a: "1",
      b: "3",
    });
  });

  // Half-translated is the normal state of a locale someone contributed; it has to degrade to
  // Vietnamese rather than to key names.
  it("shows the source language for a key a locale has not translated", () => {
    const catalog = catalogFor("en", { "brand.new": "only here" });
    expect(catalog["brand.new"]).toBe("only here");
    expect(catalog["nav.record"]).toBe("Record");
  });

  it("puts a user file above the built-in translation", () => {
    const catalog = catalogFor("en", { "nav.record": "Capture" });
    expect(catalog["nav.record"]).toBe("Capture");
  });

  it("falls a regional tag back to its primary language", () => {
    expect(catalogFor("en-GB")["nav.record"]).toBe("Record");
  });

  it("falls an unknown locale all the way back to the source", () => {
    expect(catalogFor("xx-YY")["nav.record"]).toBe("Ghi");
  });
});

describe("detectLocale", () => {
  const available = ["vi", "en"];

  it("prefers a saved choice", () => {
    expect(detectLocale(available, "en")).toBe("en");
  });

  it("ignores a saved choice for a language that is no longer installed", () => {
    expect(detectLocale(available, "ja")).not.toBe("ja");
  });

  // The browser still wins: a Vietnamese machine opens in Vietnamese. This only settles the case
  // where the browser asks for a language Summo does not have.
  it("falls back to English when the browser asks for something we do not have", () => {
    expect(detectLocale(["vi", "en"], null)).toBe(
      typeof navigator !== "undefined" && navigator.languages?.some((l) => l.startsWith("vi"))
        ? "vi"
        : "en",
    );
  });

  it("still falls back to the source language when English is not installed", () => {
    expect(detectLocale(["vi"], null)).toBe("vi");
  });
});

describe("defaults", () => {
  it("ships the default language", () => {
    expect(Object.keys(BUILT_IN)).toContain(DEFAULT);
  });
});

describe("the shipped catalogs", () => {
  const viKeys = Object.keys(flatten(viJson)).sort();
  const enKeys = Object.keys(flatten(en)).sort();

  // The failure this catches: a string added to one file and forgotten in the other, which shows up
  // as English text in a Vietnamese screen and is never noticed by the person who wrote it.
  it("English covers every Vietnamese key", () => {
    const missing = viKeys.filter((key) => !enKeys.includes(key));
    expect(missing).toEqual([]);
  });

  it("has no English key that Vietnamese lacks", () => {
    const extra = enKeys.filter((key) => !viKeys.includes(key));
    expect(extra).toEqual([]);
  });

  it("uses the same placeholders in both languages", () => {
    const placeholders = (text: string) => (text.match(/\{(\w+)\}/g) ?? []).sort();
    const viFlat = flatten(viJson);
    const enFlat = flatten(en);
    for (const key of viKeys) {
      expect(placeholders(enFlat[key] ?? ""), `placeholders differ for ${key}`).toEqual(
        placeholders(viFlat[key] ?? ""),
      );
    }
  });

  it("registers both languages as built in", () => {
    expect(Object.keys(BUILT_IN).sort()).toEqual(["en", "vi"]);
  });
});
