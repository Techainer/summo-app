import { beforeAll, describe, expect, it, vi as vitest } from "vitest";

import {
  BUILT_IN_CODES,
  BUILT_IN_LANGUAGES,
  DEFAULT,
  SOURCE,
  catalogFor,
  detectLocale,
  ensure,
  flatten,
  interpolate,
  layersOf,
  mergeCatalogs,
  plural,
  ready,
  translator,
} from ".";
import enCore from "./en.json";
import enMore from "./en.more.json";
import jaCore from "./ja.json";
import jaMore from "./ja.more.json";
import viCore from "./vi.json";
import viMore from "./vi.more.json";
import zhCore from "./zh.json";
import zhMore from "./zh.more.json";

// Each language is one catalogue in two files — eager and lazy, see `split.test.ts`. Every
// assertion below is about coverage, and a key is covered wherever it lives.
const enJson = { ...enCore, ...enMore };
const jaJson = { ...jaCore, ...jaMore };
const viJson = { ...viCore, ...viMore };
const zhJson = { ...zhCore, ...zhMore };

/**
 * The catalogues, read from disk rather than through the loader.
 *
 * These tests are about the *files* — whether `ja.json` covers every key `vi.json` has. Asking the
 * loader would make them depend on a fetch having happened, and would let a broken loader read as
 * a translation problem.
 */
const SHIPPED: Record<string, unknown> = { vi: viJson, en: enJson, ja: jaJson, zh: zhJson };

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
  // `catalogFor` reads what has been loaded, and nothing has been until something asks. Every
  // assertion below is about layering, not about fetching, so the fetching happens once up here.
  beforeAll(async () => {
    await Promise.all(BUILT_IN_CODES.map((code) => ensure(code)));
  });

  it("layers later catalogs over earlier ones", () => {
    expect(mergeCatalogs({ a: "1", b: "2" }, { b: "3" })).toEqual({
      a: "1",
      b: "3",
    });
  });

  // Half-translated is the normal state of a locale someone contributed; it has to degrade to
  // Vietnamese rather than to key names. A contributed locale is one that is *not* shipped, which
  // is why this asks about `ko` rather than `en` — a shipped catalog has every key by test.
  it("shows the source language for a key a contributed locale has not translated", () => {
    const catalog = catalogFor("ko", { "brand.new": "chỉ ở đây" });
    expect(catalog["brand.new"]).toBe("chỉ ở đây");
    expect(catalog["nav.record"]).toBe("Ghi");
  });

  it("puts a user file above a shipped translation", () => {
    expect(catalogFor("en", { "nav.record": "Capture" })["nav.record"]).toBe("Capture");
  });

  /**
   * A shipped language does not carry Vietnamese underneath it.
   *
   * This is the saving that splitting the catalogues is for, and it is only sound because the
   * parity tests below hold every shipped file to the source's keys in both directions. If that
   * ever stops being true, a Japanese screen would render key names rather than Vietnamese — so
   * the assumption is asserted here, next to the thing that depends on it.
   */
  it("needs only its own file for a shipped language", () => {
    expect(layersOf("ja")).toEqual(["ja"]);
    expect(layersOf("ja-JP")).toEqual(["ja"]);
    expect(layersOf("ko")).toEqual([SOURCE]);
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
    expect(BUILT_IN_CODES).toContain(DEFAULT);
  });
});

describe("loading a catalog", () => {
  it("has the strings once it has been fetched", async () => {
    await ensure("ja");
    expect(ready("ja")).toBe(true);
    expect(catalogFor("ja")["nav.record"]).toBe(flatten(jaJson)["nav.record"]);
  });

  // `ensure` resolving is what lets the provider stop waiting; a language nothing can serve has to
  // reach that point too, or the app renders nothing at all rather than rendering key names.
  it("resolves for a language it cannot serve, rather than hanging", async () => {
    await expect(ensure("kl")).resolves.toBeUndefined();
  });

  // Every layer has to be a file that exists, or `ready` waits on a fetch that will never happen.
  it("only ever asks for catalogs it ships", () => {
    for (const locale of [...BUILT_IN_CODES, "en-GB", "zh-Hant", "ko", "xx-YY"]) {
      expect(layersOf(locale).every((code) => BUILT_IN_CODES.includes(code))).toBe(true);
    }
  });
});

describe("the shipped catalogs", () => {
  const viFlat = flatten(viJson);
  const viKeys = Object.keys(viFlat).sort();
  const placeholders = (text: string) => (text.match(/\{(\w+)\}/g) ?? []).sort();

  // Every shipped language is held to the source, not just English. This used to compare vi to en
  // by name, so `ja.json` and `zh.json` could have been half a catalog each and nothing would have
  // said so — the app would simply have rendered Vietnamese in the gaps, on a Japanese screen,
  // where the one person who could notice cannot read it.
  const shipped = Object.entries(SHIPPED)
    .filter(([code]) => code !== SOURCE)
    .map(([code, json]) => [code, flatten(json)] as const);

  it("ships more than the source language", () => {
    expect(shipped.length).toBeGreaterThan(0);
  });

  for (const [code, catalog] of shipped) {
    const keys = Object.keys(catalog).sort();

    // The failure this catches: a string added to one file and forgotten in the others, which
    // shows up as Vietnamese text in a Japanese screen and is never noticed by whoever wrote it.
    it(`${code} covers every Vietnamese key`, () => {
      expect(viKeys.filter((key) => !keys.includes(key))).toEqual([]);
    });

    it(`${code} has no key Vietnamese lacks`, () => {
      expect(keys.filter((key) => !viKeys.includes(key))).toEqual([]);
    });

    // A dropped `{count}` renders a sentence with a hole in it, and a *renamed* one renders the
    // brace itself. Neither is visible to anyone reviewing a language they do not read.
    it(`${code} uses the same placeholders as the source`, () => {
      for (const key of viKeys) {
        expect(placeholders(catalog[key] ?? ""), `placeholders differ for ${key}`).toEqual(
          placeholders(viFlat[key] ?? ""),
        );
      }
    });
  }

  it("registers every shipped language as built in", () => {
    expect(BUILT_IN_CODES.slice().sort()).toEqual(["en", "ja", "vi", "zh"]);
    // The loader and the files it is checked against are two lists, and a language added to one
    // and not the other is either an untested catalogue or a fetch of a file nobody verified.
    expect(Object.keys(SHIPPED).sort()).toEqual(BUILT_IN_CODES.slice().sort());
  });

  // The picker is a separate list from the catalogs, and a language present in one and absent from
  // the other is either a dead entry or a translation nobody can reach.
  it("offers exactly the built-in catalogs in the picker", () => {
    expect(BUILT_IN_LANGUAGES.map((l) => l.code).sort()).toEqual(BUILT_IN_CODES.slice().sort());
  });

  // "Japanese" in a list a Japanese speaker is scanning is the one word they cannot use to find it.
  it("names each language in its own language", () => {
    for (const language of BUILT_IN_LANGUAGES) {
      expect(language.label, `${language.code} is labelled with its own tag`).not.toBe(
        language.code,
      );
    }
  });
});
