import { afterEach, describe, expect, it } from "vitest";
import { deviceWarning, handshakeFromLocation, type SessionState } from "./session";

const base: SessionState = {
  recording: false,
  connection: "closed",
  error: null,
  deviceLabel: null,
  sampleRate: null,
};

describe("device warnings", () => {
  it("flags a narrowband device with its actual rate", () => {
    // A headset in telephony mode is the single largest quality cliff in the pipeline.
    const warning = deviceWarning({ ...base, sampleRate: 8000 });
    expect(warning).toMatch(/8 kHz/);
  });

  it("flags a Bluetooth device by name even when the rate looks fine", () => {
    const warning = deviceWarning({ ...base, deviceLabel: "AirPods Pro", sampleRate: 48000 });
    expect(warning).toMatch(/Bluetooth/);
  });

  it("says nothing about an ordinary microphone", () => {
    expect(
      deviceWarning({ ...base, deviceLabel: "MacBook Pro Microphone", sampleRate: 48000 }),
    ).toBeNull();
  });
});

describe("handshake from the url", () => {
  it("reads a port and token", () => {
    expect(handshakeFromLocation("?port=54321&token=abc")).toEqual({ port: 54321, token: "abc" });
  });

  it("refuses anything incomplete or nonsensical", () => {
    for (const search of ["", "?port=54321", "?token=abc", "?port=0&token=abc", "?port=x&token=a"]) {
      expect(handshakeFromLocation(search), `should have refused ${search}`).toBeNull();
    }
  });
});

describe("the injected handshake", () => {
  const clear = () => delete (globalThis as { __SUMMO__?: unknown }).__SUMMO__;

  afterEach(clear);

  // A token in a URL lands in history, in the window title, and in whatever gets pasted into a bug
  // report. When the daemon serves the page it can do better, and does.
  it("is preferred over the query string", () => {
    (globalThis as { __SUMMO__?: unknown }).__SUMMO__ = { port: 9000, token: "injected" };
    expect(handshakeFromLocation("?port=1&token=fromurl")).toEqual({
      port: 9000,
      token: "injected",
    });
  });

  // The Tauri webview is loaded from a custom scheme; there is nowhere to inject into.
  it("falls back to the query string when nothing was injected", () => {
    clear();
    expect(handshakeFromLocation("?port=8710&token=abc")).toEqual({ port: 8710, token: "abc" });
  });

  it("ignores a half-written injection rather than connecting to nowhere", () => {
    (globalThis as { __SUMMO__?: unknown }).__SUMMO__ = { port: 9000 };
    expect(handshakeFromLocation("?port=8710&token=abc")).toEqual({ port: 8710, token: "abc" });
  });

  it("ignores an injection that is not an object", () => {
    (globalThis as { __SUMMO__?: unknown }).__SUMMO__ = "nope";
    expect(handshakeFromLocation("")).toBeNull();
  });
});
