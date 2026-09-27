import { describe, expect, it } from "vitest";

import { betterFor, missingFor, quality, type Language } from "./languages";

const language = (over: Partial<Language> = {}): Language => ({
  code: "vi",
  model: "gipformer-65m",
  model_name: "Gipformer 65M",
  size_bytes: 73_000_000,
  installed: false,
  accuracy: 0.91,
  live: true,
  multilingual_only: false,
  serving: "whisper-tiny",
  serving_name: "Whisper tiny",
  serving_accuracy: 0.34,
  ...over,
});

describe("when a better model is worth offering", () => {
  it("offers one only when the gap is worth a download", () => {
    expect(betterFor(language())).toBeDefined();
    // Four points. Below the floor, and the two numbers come from different benchmark runs anyway.
    expect(betterFor(language({ accuracy: 0.38, serving_accuracy: 0.34 }))).toBeUndefined();
  });

  it("offers nothing when the best model is the one already running", () => {
    expect(betterFor(language({ serving: "gipformer-65m" }))).toBeUndefined();
    expect(betterFor(language({ installed: true }))).toBeUndefined();
  });
});

describe("when nothing installed can hear the language at all", () => {
  /**
   * The gap `betterFor` cannot see, and the one that leaves the user worst off: they have chosen a
   * language, the panel says nothing here can transcribe it, and there is no button — while the
   * fix is one download away and the screen does not mention it.
   */
  it("offers the download that would make the language work", () => {
    const orphan = language({ serving: null, serving_name: null, serving_accuracy: 0 });
    expect(missingFor(orphan)?.model).toBe("gipformer-65m");
    // And `betterFor` still says nothing about it, which is why this function exists.
    expect(betterFor(orphan)).toBeUndefined();
  });

  /** Two recommendation panels for one language is one panel too many. */
  it("says nothing when something is already serving the language", () => {
    expect(missingFor(language())).toBeUndefined();
  });

  /**
   * No accuracy floor here, unlike `betterFor`. Weighing five points of improvement against a
   * download is a judgement call; weighing "a transcript" against "no transcript" is not.
   */
  it("offers a poor model over no model", () => {
    const orphan = language({
      serving: null,
      serving_accuracy: 0,
      accuracy: 0.31,
      model: "whisper-tiny",
    });
    expect(missingFor(orphan)?.model).toBe("whisper-tiny");
  });

  /** A button here would promise a download that does not exist. */
  it("offers nothing when the registry has nothing for the language either", () => {
    expect(missingFor(language({ serving: null, model: null }))).toBeUndefined();
    expect(missingFor(undefined)).toBeUndefined();
  });

  /**
   * Installed and not serving is a state worth not guessing about — a model on disk that the
   * ranking declined to use. Offering to download what is already there would be a button that
   * changes nothing.
   */
  it("offers nothing for a model that is already on the machine", () => {
    expect(missingFor(language({ serving: null, installed: true }))).toBeUndefined();
  });
});

describe("grading the model that is actually listening", () => {
  const vietnamese = (over: Partial<Language>): Language =>
    ({
      code: "vi",
      name: "Tiếng Việt",
      accuracy: 0.917,
      installed: false,
      multilingual_only: false,
      model: "gipformer-65m",
      model_name: "Gipformer",
      serving: "whisper-tiny",
      serving_name: "Whisper tiny",
      serving_accuracy: 0.247,
      ...over,
    }) as Language;

  /**
   * The report this exists for: Vietnamese recorded with `whisper-tiny` came back as English
   * fragments — `Xin chào tôi tên là Việt` as `Hire whats yo nam` — and the bar said nothing.
   * It was grading the Gipformer that was not installed.
   */
  it("calls a poor model poor even when a better one exists elsewhere", () => {
    expect(quality(vietnamese({}))).toBe("poor");
  });

  it("calls a good model good", () => {
    expect(quality(vietnamese({ serving: "gipformer-65m", serving_accuracy: 0.917 }))).toBe("good");
  });

  /** Nothing listening yet: grade the language, not a model that is not there. */
  it("falls back to the language's own number when nothing is serving", () => {
    expect(quality(vietnamese({ serving: null, serving_accuracy: 0 }))).toBe("good");
  });

  /** A model with no measurement is not "poor" — that is a claim about a number nobody has. */
  it("says unmeasured rather than guessing", () => {
    expect(quality(vietnamese({ serving_accuracy: 0, accuracy: 0 }))).toBe("unmeasured");
  });
});
