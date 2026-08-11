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
    expect(normalize({ translateTo: "  en " }).translateTo).toBe("en");
  });

  it("treats a non-string language as off", () => {
    expect(normalize({ translateTo: 7 as never }).translateTo).toBe("");
  });
});

describe("storage", () => {
  it("round-trips a choice", () => {
    save({ lanes: ["system"], translateTo: "en" });
    expect(load()).toEqual({ lanes: ["system"], translateTo: "en" });
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
    expect(translating({ lanes: ["mic"], translateTo: "" })).toBe(false);
    expect(translating({ lanes: ["mic"], translateTo: "en" })).toBe(true);
  });

  // Translating the microphone lane translates *you*. It is what happens when the system-audio
  // switch is forgotten, and it looks like the feature is broken.
  it("knows when nothing but the local user will be heard", () => {
    expect(hearsOthers({ lanes: ["mic"], translateTo: "en" })).toBe(false);
    expect(hearsOthers({ lanes: ["mic", "system"], translateTo: "en" })).toBe(true);
  });
});
