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

const { DEFAULT, hearsOthers, load, normalize, save, setSystemAudio, translating } =
  await import("./capture");
type Lane = "mic" | "system";

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
    save({ lanes: ["system"], translateInto: ["en", "ja"], spoken: ["vi"], device: "mic-7" });
    expect(load()).toEqual({
      lanes: ["system"],
      translateInto: ["en", "ja"],
      spoken: ["vi"],
      device: "mic-7",
    });
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
    expect(translating({ lanes: ["mic"], translateInto: [], spoken: [], device: "" })).toBe(false);
    expect(translating({ lanes: ["mic"], translateInto: ["en"], spoken: [], device: "" })).toBe(
      true,
    );
  });

  // Translating the microphone lane translates *you*. It is what happens when the system-audio
  // switch is forgotten, and it looks like the feature is broken.
  it("knows when nothing but the local user will be heard", () => {
    expect(hearsOthers({ lanes: ["mic"], translateInto: ["en"], spoken: [], device: "" })).toBe(
      false,
    );
    expect(
      hearsOthers({ lanes: ["mic", "system"], translateInto: ["en"], spoken: [], device: "" }),
    ).toBe(true);
  });
});

describe("the spoken language", () => {
  /// An older build wrote no `spoken` at all, and a capture read back without one must record in
  /// whatever the daemon's settings say rather than refusing or guessing a language.
  it("defaults to empty, which the daemon reads as its own setting", () => {
    expect(normalize({ lanes: ["mic"], translateInto: [] }).spoken).toEqual([]);
    expect(DEFAULT.spoken).toEqual([]);
  });

  /**
   * A bare string is what every browser that has ever run this has in storage. Dropping it would
   * silently reset the spoken language for all of them, at the start of their next meeting, with
   * nothing on screen to say why — the same reason `translateTo` is still read.
   */
  it("reads the single string this used to be", () => {
    expect(normalize({ spoken: "vi" as never }).spoken).toEqual(["vi"]);
    store.set("summo.capture", JSON.stringify({ lanes: ["mic"], spoken: "en" }));
    expect(load().spoken).toEqual(["en"]);
  });

  /**
   * Order is the answer to "which is it mostly in", and the daemon pairs the specialist for the
   * first. Losing it would make a bilingual meeting pick its second model at random.
   */
  it("keeps the order and drops duplicates and blanks", () => {
    expect(normalize({ spoken: ["vi", "EN", " ", "vi"] }).spoken).toEqual(["vi", "en"]);
  });

  /// Codes are compared against the manifests' own spelling, where they are lower case.
  it("is normalised, so `VI ` from an older build still matches a model", () => {
    expect(normalize({ spoken: " VI " as never }).spoken).toEqual(["vi"]);
    expect(normalize({ spoken: 7 as never }).spoken).toEqual([]);
  });
});

describe("system audio has one home", () => {
  /**
   * It had two. The settings screen wrote `recording.capture_system_audio` into the daemon and a
   * recording read `lanes` out of `localStorage`, so the switch labelled "capture system audio"
   * moved a number nothing consulted — a control that does nothing, which is worse than one that
   * is missing.
   */
  it("adds and removes the system lane", () => {
    const mic = { ...DEFAULT, lanes: ["mic"] as Lane[] };
    expect(setSystemAudio(mic, true).lanes).toEqual(["mic", "system"]);
    expect(setSystemAudio(setSystemAudio(mic, true), false).lanes).toEqual(["mic"]);
  });

  /** Turning it on twice is one lane, not two. */
  it("does not add the lane twice", () => {
    const both = { ...DEFAULT, lanes: ["mic", "system"] as Lane[] };
    expect(setSystemAudio(both, true).lanes).toEqual(["mic", "system"]);
  });

  /**
   * The daemon refuses a session with no lanes, so turning system audio off on a system-only
   * capture has to leave the microphone rather than nothing — otherwise this switch becomes a
   * record button that fails.
   */
  it("never leaves a capture with nothing to hear", () => {
    const only = { ...DEFAULT, lanes: ["system"] as Lane[] };
    expect(setSystemAudio(only, false).lanes).toEqual(["mic"]);
  });
});

describe("which microphone", () => {
  /**
   * `Microphone` has accepted a `deviceId` since it was written and nothing ever passed one, while
   * `recording.device_id` sat in the settings file being saved and read by nobody. Somebody with a
   * headset and a built-in microphone could name the one they wanted and be recorded by the other,
   * with a settings screen showing their choice the whole time.
   */
  it("is kept beside the lanes, because the recording needs it before any network call", () => {
    save({ ...DEFAULT, device: "abc123" });
    expect(load().device).toBe("abc123");
  });

  it("is empty for whatever the system calls default", () => {
    expect(DEFAULT.device).toBe("");
    expect(normalize({ device: "   " }).device).toBe("");
    expect(normalize({}).device).toBe("");
  });

  /**
   * Not lower-cased, unlike `spoken`. A `deviceId` is an opaque token the browser minted; changing
   * its case names a different device, or none.
   */
  it("keeps the exact token the browser gave, case and all", () => {
    expect(normalize({ device: "  AbC-123  " }).device).toBe("AbC-123");
  });

  it("survives a stored value from a build that had no such field", () => {
    store.set("summo.capture", JSON.stringify({ lanes: ["mic"], spoken: ["vi"] }));
    expect(load().device).toBe("");
  });
});
