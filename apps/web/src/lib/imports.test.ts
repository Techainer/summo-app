import { describe as group, expect, it } from "vitest";

import { baseName, describe, isFinished, percent, type Job } from "./imports";

const job = (over: Partial<Job> = {}): Job => ({
  id: "j1",
  title: "Họp tuần",
  source: "/home/a/hop.mp4",
  state: "queued",
  ...over,
});

group("percent", () => {
  it("is null before there is anything to measure", () => {
    expect(percent(job())).toBeNull();
    expect(percent(job({ state: "extracting" }))).toBeNull();
  });

  it("reads from the audio consumed, not from the segments found", () => {
    expect(percent(job({ state: "running", done_s: 30, total_s: 120, segments: 0 }))).toBe(25);
  });

  // A bar sitting at 0% looks broken, and a file whose length ffmpeg could not report is exactly
  // when a long import is most likely to worry the user.
  it("is null rather than zero when the length is unknown", () => {
    expect(percent(job({ state: "running", done_s: 30, total_s: 0 }))).toBeNull();
  });

  it("cannot exceed 100 when the last block overshoots the header's length", () => {
    expect(percent(job({ state: "running", done_s: 121, total_s: 120 }))).toBe(100);
  });

  it("is 100 once done, even though a finished job carries no progress fields", () => {
    expect(percent(job({ state: "done", meeting: "m1" }))).toBe(100);
  });
});

group("describe", () => {
  it("mentions how many sentences have landed, so a slow import still looks alive", () => {
    const text = describe(job({ state: "running", done_s: 60, total_s: 120, segments: 42 }));
    expect(text).toContain("50%");
    expect(text).toContain("42");
  });

  it("omits the count before the first sentence rather than saying 0 câu", () => {
    expect(describe(job({ state: "running", done_s: 1, total_s: 120, segments: 0 }))).toBe(
      "Đang nhận dạng — 1%",
    );
  });

  it("shows the daemon's own error rather than a generic one", () => {
    expect(describe(job({ state: "failed", error: "không có âm thanh" }))).toBe(
      "không có âm thanh",
    );
  });

  it("still says something when a failure arrived with no message", () => {
    expect(describe(job({ state: "failed" }))).toBe("Lỗi");
  });
});

group("isFinished", () => {
  it("treats a failure as settled, so polling stops", () => {
    expect(isFinished(job({ state: "failed", error: "x" }))).toBe(true);
    expect(isFinished(job({ state: "done" }))).toBe(true);
    expect(isFinished(job({ state: "extracting" }))).toBe(false);
    expect(isFinished(job({ state: "queued" }))).toBe(false);
  });
});

group("baseName", () => {
  it("handles a Windows path, which arrives whole over the socket", () => {
    expect(baseName("C:\\Users\\a\\Videos\\hop.mp4")).toBe("hop.mp4");
  });

  it("handles a posix path", () => {
    expect(baseName("/home/a/hop.mp4")).toBe("hop.mp4");
  });

  it("survives a trailing separator instead of returning empty", () => {
    expect(baseName("/home/a/")).toBe("a");
  });
});
