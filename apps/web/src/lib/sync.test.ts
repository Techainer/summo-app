import { describe, expect, it } from "vitest";

import { actionKey, isQuiet, parts, total, type Step, type Summary } from "./sync";

const summary = (over: Partial<Summary> = {}): Summary => ({
  uploaded: 0,
  downloaded: 0,
  merged: 0,
  deleted: 0,
  resurrected: 0,
  ...over,
});

describe("what a sync reports", () => {
  /**
   * The zeroes are the noise. "0 uploaded, 0 downloaded, 3 merged, 0 deleted, 0 restored" makes
   * the reader hunt for the one number that is not zero — which is the whole of what they came to
   * find out.
   */
  it("drops the counts that are zero", () => {
    expect(parts(summary({ merged: 3 }))).toEqual([{ key: "sync.merged", count: 3 }]);
    expect(parts(summary({ uploaded: 2, deleted: 1 }))).toEqual([
      { key: "sync.uploaded", count: 2 },
      { key: "sync.deleted", count: 1 },
    ]);
  });

  it("knows when there was nothing to do", () => {
    expect(isQuiet(summary())).toBe(true);
    expect(parts(summary())).toEqual([]);
    expect(isQuiet(summary({ resurrected: 1 }))).toBe(false);
    expect(total(summary({ uploaded: 1, downloaded: 2, merged: 3 }))).toBe(6);
  });
});

describe("what each step is called", () => {
  const step = (over: Partial<Step>): Step => ({ path: "vault/a.md", action: "upload", ...over });

  /**
   * These strings come off the wire from `summo_sync::plan::Action`, which is
   * `rename_all = "snake_case"` and flattened into the step. `session.rs` has a test pinning the
   * same five names from the other side — that pair is the whole guard against a rename that is
   * invisible to every Rust caller and turns a plan into a column of "unknown" here.
   */
  it("matches the names the daemon puts on the wire", () => {
    expect(actionKey(step({ action: "upload" }))).toBe("sync.action_upload");
    expect(actionKey(step({ action: "download" }))).toBe("sync.action_download");
    expect(actionKey(step({ action: "merge" }))).toBe("sync.action_merge");
    expect(actionKey(step({ action: "delete_remote" }))).toBe("sync.action_delete_there");
    expect(actionKey(step({ action: "delete_local" }))).toBe("sync.action_delete_here");
  });

  /**
   * A file coming back from the dead is confusing unless somebody says which side kept it. The
   * edit that survived was made *there*, so the file is restored *here* — and getting that
   * backwards tells the reader their own edit was the one thrown away.
   */
  it("says which side a restored file is coming back on", () => {
    expect(actionKey(step({ action: "resurrect", edited_on: "local" }))).toBe(
      "sync.action_restore_there",
    );
    expect(actionKey(step({ action: "resurrect", edited_on: "remote" }))).toBe(
      "sync.action_restore_here",
    );
  });

  /** A newer daemon with an action this build has never heard of must still draw a row. */
  it("does not blank a row for an action it does not know", () => {
    expect(actionKey(step({ action: "teleport" as never }))).toBe("sync.action_unknown");
  });
});
