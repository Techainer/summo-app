import { beforeEach, describe, expect, it } from "vitest";

/**
 * A minimal `window.localStorage`.
 *
 * The suite runs in node, and adding jsdom to get one key-value store would be a large dependency
 * for a small need. This is the entire surface `capture.ts` touches, so the stub exercises the real
 * code path rather than a mock of it. Installed before the module is imported.
 */
const store = new Map<string, string>();
(globalThis as { window?: unknown }).window = {
  localStorage: {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, v),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  },
};

const { DEFAULT, hearsOthers, load, normalize, save, translating } = await import("./capture");

beforeEach(() => store.clear());

describe("normalize", () => {
  // A session with no lanes is rejected by the daemon, so a corrupt preference would turn into a
  // record button that fails.
  it("never produces a capture with no lanes", () => {
    expect(normalize({ lanes: [] }).lanes).toEqual(DEFAULT.lanes);
    expect(normalize({}).lanes).toEqual(DEFAULT.lanes);
    expect(normalize(null).lanes).toEqual(DEFAULT.lanes);
  });

  it("drops lane names it does not recognise", () => {
    expect(normalize({ lanes: ["mic", "speakerphone", "system"] as never }).lanes).toEqual([
      "mic",
      "system",
    ]);
  });

  it("collapses a duplicated lane, which would open the same capture twice", () => {
    expect(normalize({ lanes: ["mic", "mic"] }).lanes).toEqual(["mic"]);
  });

  it("trims a language tag rather than sending whitespace to the daemon", () => {
    expect(normalize({ translateInto: ["  en "] }).translateInto).toEqual(["en"]);
  });

  it("treats a non-string language as off", () => {
    expect(normalize({ translateInto: 7 as never }).translateInto).toEqual([]);
    expect(normalize({ translateInto: [7 as never] }).translateInto).toEqual([]);
  });

  it("drops a language asked for twice, which would subtitle every line twice", () => {
    expect(normalize({ translateInto: ["en", "en", "ja"] }).translateInto).toEqual(["en", "ja"]);
  });

  /**
   * `translateTo` was a single string and is in every existing browser's local storage. Reading it
   * back as nothing would turn translation off for everybody who had it on, at the start of their
   * next meeting, with nothing on screen to say why.
   */
  it("reads the single language older versions saved", () => {
    expect(normalize({ translateTo: "en" } as never).translateInto).toEqual(["en"]);
    expect(normalize({ translateTo: "" } as never).translateInto).toEqual([]);
  });
});

describe("storage", () => {
  it("round-trips a choice", () => {
    save({ lanes: ["system"], translateInto: ["en", "ja"], spoken: "vi" });
    expect(load()).toEqual({ lanes: ["system"], translateInto: ["en", "ja"], spoken: "vi" });
  });

  it("falls back to the default when nothing was saved", () => {
    expect(load()).toEqual(DEFAULT);
  });

  // Parsed from storage an older version or a user wrote; a bad value must not stop recording.
  it("falls back to the default on unparseable storage", () => {
    store.set("summo.capture", "{ not json");
    expect(load()).toEqual(DEFAULT);
  });

  it("repairs a stored value that is valid json but nonsense", () => {
    store.set("summo.capture", JSON.stringify({ lanes: "mic" }));
    expect(load()).toEqual(DEFAULT);
  });
});

describe("what the capture means", () => {
  it("knows when live translation is on", () => {
    expect(translating({ lanes: ["mic"], translateInto: [], spoken: "" })).toBe(false);
    expect(translating({ lanes: ["mic"], translateInto: ["en"], spoken: "" })).toBe(true);
  });

  // Translating the microphone lane translates *you*. It is what happens when the system-audio
  // switch is forgotten, and it looks like the feature is broken.
  it("knows when nothing but the local user will be heard", () => {
    expect(hearsOthers({ lanes: ["mic"], translateInto: ["en"], spoken: "" })).toBe(false);
    expect(hearsOthers({ lanes: ["mic", "system"], translateInto: ["en"], spoken: "" })).toBe(true);
  });
});

describe("the spoken language", () => {
  /// An older build wrote no `spoken` at all, and a capture read back without one must record in
  /// whatever the daemon's settings say rather than refusing or guessing a language.
  it("defaults to empty, which the daemon reads as its own setting", () => {
    expect(normalize({ lanes: ["mic"], translateInto: [] }).spoken).toBe("");
    expect(DEFAULT.spoken).toBe("");
  });

  /// Codes are compared against the manifests' own spelling, where they are lower case.
  it("is normalised, so `VI ` from an older build still matches a model", () => {
    expect(normalize({ spoken: " VI " }).spoken).toBe("vi");
    expect(normalize({ spoken: 7 as never }).spoken).toBe("");
  });
});
