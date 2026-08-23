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

  /// The reported bug: a Mac with the microphone granted, recording happily, and a settings panel
  /// insisting the permission had never been asked for. WKWebView answers `prompt` for a permission
  /// macOS has already given, and Chromium does the same once a granted stream is released.
  ///
  /// Device labels cannot lie the same way — the specification hides them until a capture
  /// permission is granted — so a named input outranks the query.
  it("believes a named device over a browser that says prompt", async () => {
    const navigator = {
      permissions: { query: vi.fn().mockResolvedValue({ state: "prompt" }) },
      mediaDevices: {
        enumerateDevices: vi
          .fn()
          .mockResolvedValue([{ kind: "audioinput", label: "MacBook Pro Microphone" }]),
      },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("granted");
  });

  /// Safari, where the query says nothing at all. The labels are the only evidence there is.
  it("uses labels where there is no query to ask", async () => {
    const navigator = {
      mediaDevices: {
        enumerateDevices: vi.fn().mockResolvedValue([{ kind: "audioinput", label: "USB Mic" }]),
      },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("granted");
  });

  /// Unlabelled devices prove nothing: that is exactly what a browser shows *before* permission.
  /// The state has to stay whatever the query said rather than being upgraded on a device count.
  it("does not read an unnamed device as a grant", async () => {
    const navigator = {
      permissions: { query: vi.fn().mockResolvedValue({ state: "prompt" }) },
      mediaDevices: {
        enumerateDevices: vi.fn().mockResolvedValue([{ kind: "audioinput", label: "" }]),
      },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("prompt");
  });

  /// A refusal is never overridden. Labels can outlive a permission that has since been revoked,
  /// and reporting "granted" there would hide the recovery steps from the one person who needs
  /// them.
  it("never talks a refusal up into a grant", async () => {
    const navigator = {
      permissions: { query: vi.fn().mockResolvedValue({ state: "denied" }) },
      mediaDevices: {
        enumerateDevices: vi.fn().mockResolvedValue([{ kind: "audioinput", label: "Built-in" }]),
      },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("denied");
  });

  /// An output device is not an input. Speaker labels are exposed on the same call and would be a
  /// grant this app never asked for.
  it("ignores everything that is not an audio input", async () => {
    const navigator = {
      permissions: { query: vi.fn().mockResolvedValue({ state: "prompt" }) },
      mediaDevices: {
        enumerateDevices: vi
          .fn()
          .mockResolvedValue([{ kind: "audiooutput", label: "External Headphones" }]),
      },
    } as unknown as Navigator;
    await expect(micState(navigator)).resolves.toBe("prompt");
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
