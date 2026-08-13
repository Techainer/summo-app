import { describe, expect, it, vi } from "vitest";

import {
  detectBrowser,
  detectPlatform,
  micRecovery,
  micState,
  requestMic,
  systemAudio,
} from "./permissions";

const nav = (userAgent: string, extra: Partial<Navigator> = {}) =>
  ({ userAgent, ...extra }) as Navigator;

describe("detectPlatform", () => {
  it("prefers the hint over the user-agent string", () => {
    const navigator = {
      userAgent: "Mozilla/5.0 (X11; Linux x86_64)",
      userAgentData: { platform: "macOS" },
    } as unknown as Navigator;
    expect(detectPlatform(navigator)).toBe("macos");
  });

  it("reads the platform out of a user-agent when there is no hint", () => {
    expect(detectPlatform(nav("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)"))).toBe("macos");
    expect(detectPlatform(nav("Mozilla/5.0 (Windows NT 10.0; Win64; x64)"))).toBe("windows");
    expect(detectPlatform(nav("Mozilla/5.0 (X11; Linux x86_64)"))).toBe("linux");
  });

  /// An unknown platform gets no operating-system step rather than a guess at one.
  it("says unknown rather than guessing", () => {
    expect(detectPlatform(nav("Mozilla/5.0 (PlayStation 5)"))).toBe("unknown");
    expect(micRecovery("unknown", "chrome").map((s) => s.key)).toEqual([
      "permissions.fix_browser_chrome",
      "permissions.fix_retry",
    ]);
  });
});

describe("detectBrowser", () => {
  /// Every Chromium browser claims to be Safari, and Edge claims to be Chrome. Order decides.
  it("is not fooled by the compatibility tokens every browser carries", () => {
    const chrome =
      "Mozilla/5.0 (Macintosh) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36";
    const edge = `${chrome} Edg/120`;
    const safari = "Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15";
    expect(detectBrowser(nav(chrome))).toBe("chrome");
    expect(detectBrowser(nav(edge))).toBe("chrome");
    expect(detectBrowser(nav(safari))).toBe("safari");
    expect(detectBrowser(nav("Mozilla/5.0 Gecko/20100101 Firefox/121.0"))).toBe("firefox");
  });
});

describe("micState", () => {
  it("reports what the browser says", async () => {
    const navigator = {
      permissions: { query: vi.fn().mockResolvedValue({ state: "denied" }) },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("denied");
  });

  /// Safari has no `permissions.query` for the microphone. Unknown is an answer, not a failure —
  /// and must not read as "denied", which would show a recovery panel to somebody who has not been
  /// asked yet.
  it("is unknown, not denied, where the browser cannot say", async () => {
    await expect(micState({} as Navigator)).resolves.toBe("unknown");
    const throws = {
      permissions: { query: vi.fn().mockRejectedValue(new Error("unsupported")) },
    } as unknown as Navigator;
    await expect(micState(throws)).resolves.toBe("unknown");
  });
});

describe("requestMic", () => {
  const withMedia = (getUserMedia: unknown) => {
    vi.stubGlobal("navigator", { mediaDevices: { getUserMedia } });
  };

  it("releases the device as soon as the permission is settled", async () => {
    const stop = vi.fn();
    withMedia(vi.fn().mockResolvedValue({ getTracks: () => [{ stop }] }));
    await expect(requestMic()).resolves.toBe("granted");
    // A held stream lights the recording indicator on a machine that is not recording.
    expect(stop).toHaveBeenCalledOnce();
    vi.unstubAllGlobals();
  });

  /// A refusal and a broken device need different advice, so they must not collapse into one state.
  it("separates a refusal from a device that will not open", async () => {
    withMedia(vi.fn().mockRejectedValue(new DOMException("no", "NotAllowedError")));
    await expect(requestMic()).resolves.toBe("denied");

    withMedia(vi.fn().mockRejectedValue(new DOMException("busy", "NotReadableError")));
    await expect(requestMic()).resolves.toBe("unknown");
    vi.unstubAllGlobals();
  });
});

describe("micRecovery", () => {
  /// The browser's own block is more common and easier to undo than the operating system's, and a
  /// user sent to System Settings for a permission already granted stops trusting the instructions.
  it("puts the browser before the operating system", () => {
    expect(micRecovery("macos", "safari").map((s) => s.key)).toEqual([
      "permissions.fix_browser_safari",
      "permissions.fix_macos",
      "permissions.fix_retry",
    ]);
  });

  /// macOS grants microphone access to the application, and that application is the browser.
  it("names the application macOS will list", () => {
    const [, macos] = micRecovery("macos", "firefox");
    expect(macos).toEqual({ key: "permissions.fix_macos", values: { app: "Firefox" } });
  });
});

describe("systemAudio", () => {
  it("is honest about macOS having no route", () => {
    expect(systemAudio("macos").supported).toBe(false);
    expect(systemAudio("linux").supported).toBe(true);
    expect(systemAudio("windows").supported).toBe(true);
  });
});
