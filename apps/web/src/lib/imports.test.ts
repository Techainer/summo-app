import { describe as group, expect, it } from "vitest";

import { ImportClient, baseName, describe, isFinished, percent, type Job } from "./imports";

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
  it("reports a percentage once the length is known", () => {
    expect(describe(job({ state: "running", done_s: 60, total_s: 120, segments: 42 }))).toEqual({
      key: "import.state_running_pct",
      values: { percent: 50 },
    });
  });

  it("drops to the plain running key when the length is unknown", () => {
    expect(describe(job({ state: "running", done_s: 1, total_s: 0 }))).toEqual({
      key: "import.state_running",
      values: {},
    });
  });

  // "không có âm thanh" tells the user what to fix; "Lỗi" does not.
  it("passes the daemon's own message through instead of a generic one", () => {
    expect(describe(job({ state: "failed", error: "không có âm thanh" }))).toEqual({
      text: "không có âm thanh",
    });
  });

  it("still says something when a failure arrived with no message", () => {
    expect(describe(job({ state: "failed" }))).toEqual({
      key: "import.state_failed",
      values: {},
    });
  });

  it("carries the sentence count once done", () => {
    expect(describe(job({ state: "done", segments: 7 }))).toEqual({
      key: "import.state_done",
      values: { count: 7 },
    });
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

group("what the daemon is actually sent", () => {
  /**
   * `keepSource` is this codebase's spelling and `keep_source` is the daemon's. Serde drops a field
   * it does not know without complaining, so sending the wrong one is accepted, ignored, and looks
   * from the outside exactly like it worked — the import runs, and the copy the user asked for is
   * simply never made. That failure is invisible until the original file moves, months later.
   */
  it("renames keepSource to the field the daemon reads", async () => {
    const sent: string[] = [];
    const original = globalThis.fetch;
    globalThis.fetch = ((_url: string, init?: RequestInit) => {
      sent.push(typeof init?.body === "string" ? init.body : "");
      return Promise.resolve(
        new Response("{}", { status: 200, headers: { "content-type": "application/json" } }),
      );
    }) as typeof fetch;

    try {
      const client = new ImportClient({ port: 1, token: "t" });
      await client.start("/a/b.mp4", { keepSource: true });
      await client.start("/a/b.mp4");

      expect(JSON.parse(sent[0]!)).toMatchObject({ path: "/a/b.mp4", keep_source: true });
      // Explicitly false rather than absent: the daemon's default is false either way, and a field
      // that is present says what this client believes rather than leaving it to be inferred.
      expect(JSON.parse(sent[1]!)).toMatchObject({ keep_source: false });
      expect(JSON.parse(sent[0]!)).not.toHaveProperty("keepSource");
    } finally {
      globalThis.fetch = original;
    }
  });
});
