import { describe, expect, it } from "vitest";

import type { CatalogueModel } from "./catalogue";
import { covers, describe as narrate, isFinished, percent, voicesFor, type Job } from "./dub";

const job = (over: Partial<Job> = {}): Job => ({
  id: "j1",
  meeting: "01E2E0",
  title: "Họp tuần",
  lang: "en",
  state: "queued",
  ...over,
});

describe("the dub progress bar", () => {
  /**
   * The bar exists because two passes over an hour of transcript take minutes. What it must never
   * do is fill, reset and fill again — which is what one bar per pass would look like.
   */
  it("gives each pass half the bar, so it fills once across work that happens twice", () => {
    expect(percent(job({ state: "speaking", pass: 1, spoken: 0, total: 10 }))).toBe(0);
    expect(percent(job({ state: "speaking", pass: 1, spoken: 5, total: 10 }))).toBe(25);
    expect(percent(job({ state: "speaking", pass: 1, spoken: 10, total: 10 }))).toBe(50);
    expect(percent(job({ state: "speaking", pass: 2, spoken: 0, total: 10 }))).toBe(50);
    expect(percent(job({ state: "speaking", pass: 2, spoken: 10, total: 10 }))).toBe(100);
  });

  /** A bar at 0% for a job that has not started looks stuck; `null` draws the indeterminate sweep. */
  it("has nothing honest to show before the first line is spoken", () => {
    expect(percent(job())).toBeNull();
    expect(percent(job({ state: "loading" }))).toBeNull();
    expect(percent(job({ state: "failed", error: "no voice" }))).toBeNull();
    // Length not yet known.
    expect(percent(job({ state: "speaking", pass: 1, spoken: 0, total: 0 }))).toBeNull();
  });

  it("does not go back to zero while the takes are being laid over the recording", () => {
    expect(percent(job({ state: "mixing" }))).toBe(98);
    expect(percent(job({ state: "done", lines: 10, of: 12 }))).toBe(100);
  });

  it("knows which states have stopped moving", () => {
    expect(isFinished(job())).toBe(false);
    expect(isFinished(job({ state: "speaking" }))).toBe(false);
    expect(isFinished(job({ state: "done" }))).toBe(true);
    expect(isFinished(job({ state: "failed" }))).toBe(true);
  });
});

describe("what the row says", () => {
  /**
   * The daemon's own message names the voice that was missing *and* the ones that would have
   * worked. No wording invented here can do that, so a failure is passed through as text.
   */
  it("passes the daemon's failure through rather than replacing it", () => {
    const said = narrate(job({ state: "failed", error: "no installed voice speaks ja." }));
    expect(said).toEqual({ text: "no installed voice speaks ja." });
  });

  it("falls back to a key when a failure arrived with no message", () => {
    expect(narrate(job({ state: "failed" }))).toEqual({ key: "dub.state_failed", values: {} });
  });

  it("names the pass, because two passes with one wording reads as a stall", () => {
    const said = narrate(job({ state: "speaking", pass: 2, spoken: 3, total: 10 }));
    expect(said).toEqual({ key: "dub.state_speaking_pct", values: { pass: 2, percent: 65 } });
  });

  it("says how much of the meeting was translated, not only that it finished", () => {
    expect(narrate(job({ state: "done", lines: 40, of: 52 }))).toEqual({
      key: "dub.state_done",
      values: { count: 40, of: 52 },
    });
  });
});

describe("which voices can do the job", () => {
  const voice = (id: string, langs: string[], task: CatalogueModel["task"] = "tts") =>
    ({ id, name: id, task, langs, license: "MIT", size_bytes: 1 }) as CatalogueModel;

  /**
   * The bug this filter exists for. A VITS voice handed a language it was not trained for does not
   * refuse: it runs the text through the phoneme table it has and speaks the result, confidently,
   * over somebody's meeting.
   */
  it("leaves out a voice that speaks another language", () => {
    const installed = [voice("vits-vi", ["vi"]), voice("vits-en", ["en"])];
    expect(voicesFor(installed, "en").map((m) => m.id)).toEqual(["vits-en"]);
    expect(voicesFor(installed, "ja")).toEqual([]);
  });

  it("leaves out models that are not voices at all", () => {
    const installed = [voice("sense-voice", ["*"], "asr"), voice("vits-en", ["en"])];
    expect(voicesFor(installed, "en").map((m) => m.id)).toEqual(["vits-en"]);
  });

  /** `*` is a claim to every language, and `en-US` asks for `en`. Mirrors `langs_cover`. */
  it("honours a multilingual claim and a regional tag", () => {
    expect(covers(["*"], "ja")).toBe(true);
    expect(covers(["en"], "en-US")).toBe(true);
    expect(covers(["EN"], "en")).toBe(true);
    expect(covers(["zh"], "vi")).toBe(false);
    expect(covers([], "vi")).toBe(false);
  });
});
