import { describe, expect, it } from "vitest";

import {
  blocker,
  isFinished,
  needsConsent,
  optional,
  percent,
  preferred,
  size,
  type Check,
  type Install,
  type Recommended,
  type Status,
} from "./onboarding";

const status = (checks: Check[]): Status => ({
  recognition: true,
  acknowledged: false,
  can_record: checks.every((c) => !c.blocking || c.ready),
  fresh: true,
  should_prompt: true,
  needs_attention: checks.some((c) => c.blocking && !c.ready),
  checks,
  hardware: { cores: 8, total_ram_mb: 16_384, os: "linux", arch: "x86_64" },
});

const model = (over: Partial<Recommended> = {}): Recommended => ({
  id: "m",
  name: "M",
  score: 1,
  reason: "",
  live_capable: false,
  installed: false,
  ...over,
});

const install = (over: Partial<Install> = {}): Install => ({
  model: "m",
  name: "M",
  state: "queued",
  ...over,
});

describe("blocker", () => {
  // Telling a user who wants to record that ffmpeg is missing is true, unhelpful, and the reason
  // setup screens have four steps when they need one.
  it("ignores steps that do not stop a recording", () => {
    const s = status([
      { step: "models", ready: true, blocking: true, detail: "" },
      { step: "ffmpeg", ready: false, blocking: false, detail: "" },
      { step: "llm", ready: false, blocking: false, detail: "" },
    ]);
    expect(blocker(s)).toBeNull();
  });

  it("names the step that does", () => {
    const s = status([{ step: "models", ready: false, blocking: true, detail: "none" }]);
    expect(blocker(s)?.step).toBe("models");
  });

  it("lists the optional steps separately", () => {
    const s = status([
      { step: "models", ready: true, blocking: true, detail: "" },
      { step: "ffmpeg", ready: false, blocking: false, detail: "" },
      { step: "llm", ready: true, blocking: false, detail: "ollama" },
    ]);
    expect(optional(s).map((c) => c.step)).toEqual(["ffmpeg"]);
  });
});

describe("percent", () => {
  it("is null before the size is known", () => {
    expect(percent(install())).toBeNull();
    expect(percent(install({ state: "downloading", done: 10, total: 0 }))).toBeNull();
    expect(percent(install({ state: "installing" }))).toBeNull();
  });

  it("reads from bytes once they are known", () => {
    expect(percent(install({ state: "downloading", done: 50, total: 200 }))).toBe(25);
  });

  it("is 100 once done, even though a finished job carries no byte counts", () => {
    expect(percent(install({ state: "done" }))).toBe(100);
  });

  it("cannot exceed 100", () => {
    expect(percent(install({ state: "downloading", done: 300, total: 200 }))).toBe(100);
  });
});

describe("isFinished", () => {
  it("treats a failure as settled, so polling stops", () => {
    expect(isFinished(install({ state: "failed", error: "x" }))).toBe(true);
    expect(isFinished(install({ state: "done" }))).toBe(true);
    expect(isFinished(install({ state: "installing" }))).toBe(false);
  });
});

describe("size", () => {
  it("scales to the largest sensible unit", () => {
    expect(size(512)).toBe("512 B");
    expect(size(1024 * 1024 * 40)).toBe("40 MB");
    expect(size(1024 * 1024 * 1024 * 2)).toBe("2.0 GB");
  });

  it("renders nothing for a size the registry did not report", () => {
    expect(size(undefined)).toBe("");
    expect(size(0)).toBe("");
  });
});

describe("preferred", () => {
  // The first thing a new user does is press Record. A model that transcribes at 3× real time is a
  // bad first impression however good its accuracy.
  it("prefers one that can keep up with live audio over a higher-ranked batch model", () => {
    const models = [
      model({ id: "accurate", live_capable: false }),
      model({ id: "fast", live_capable: true }),
    ];
    expect(preferred(models)?.id).toBe("fast");
  });

  it("prefers one that is already installed over any download", () => {
    const models = [
      model({ id: "fast", live_capable: true }),
      model({ id: "here", installed: true }),
    ];
    expect(preferred(models)?.id).toBe("here");
  });

  it("falls back to the top of the ranking when nothing is live capable", () => {
    expect(preferred([model({ id: "only" })])?.id).toBe("only");
  });

  it("returns nothing when the registry is unreachable and nothing is installed", () => {
    expect(preferred([])).toBeNull();
  });
});

describe("needsConsent", () => {
  // A gated or non-redistributable model is a legitimate choice; it just has to be visible before
  // the click rather than after.
  it("flags a gated model", () => {
    expect(needsConsent(model({ gated: true }))).toBe(true);
  });

  it("flags one Summo does not redistribute", () => {
    expect(needsConsent(model({ redistributable: false }))).toBe(true);
  });

  it("leaves an ordinary permissive model alone", () => {
    expect(needsConsent(model({ gated: false, redistributable: true }))).toBe(false);
  });
});
