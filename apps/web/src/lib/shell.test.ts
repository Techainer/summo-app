import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { inShell, shellHandshake } from "./shell";

/**
 * Waiting for a shell's engine.
 *
 * The behaviour worth pinning down is what happens when it does *not* come up. The app renders
 * nothing until this resolves, so every branch that fails to settle is a permanently blank window
 * — which is what both shells did before this module existed, for a different reason.
 */

const invoke = vi.fn<(command: string) => Promise<unknown>>();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (command: string) => invoke(command) }));

/** A whole wait, in a few milliseconds. Real timers: see the note on `shellHandshake`. */
const QUICKLY = { pollMs: 1, giveUpMs: 40 };

beforeEach(() => {
  invoke.mockReset();
});

afterEach(() => {
  delete (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("inShell", () => {
  it("is false in a browser", () => {
    expect(inShell()).toBe(false);
  });

  it("is true when Tauri has injected itself", () => {
    (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    expect(inShell()).toBe(true);
  });
});

describe("shellHandshake", () => {
  it("keeps asking while the engine is still starting", async () => {
    invoke
      .mockResolvedValueOnce({ status: "starting" })
      .mockResolvedValueOnce({ status: "starting" })
      .mockResolvedValueOnce({ status: "ready", port: 8712, token: "abc" });

    await expect(shellHandshake(QUICKLY)).resolves.toEqual({ port: 8712, token: "abc" });
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("reports the daemon's own words when it fails", async () => {
    // The shell passes through whatever the engine printed before dying. A user whose port is
    // taken can act on that sentence; "could not start" is a sentence they can only report.
    invoke.mockResolvedValue({
      status: "failed",
      error: "cannot bind loopback port: Address already in use",
    });

    await expect(shellHandshake(QUICKLY)).rejects.toThrow("Address already in use");
    // Not retried: a failure is an answer, and asking the same question again for forty seconds
    // only delays the moment the user is told.
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("stops at once when the command itself is missing", async () => {
    // An older shell, or a build where the handler was never registered. Retrying cannot fix it,
    // and the wait would otherwise be a blank window before the same answer.
    invoke.mockRejectedValue(new Error("Command engine_handshake not found"));

    await expect(shellHandshake(QUICKLY)).rejects.toThrow("cannot reach its engine");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("gives up rather than waiting forever on a shell that never answers", async () => {
    invoke.mockResolvedValue({ status: "starting" });

    await expect(shellHandshake(QUICKLY)).rejects.toThrow("the engine did not start");
    expect(invoke.mock.calls.length).toBeGreaterThan(1);
  });
});
