// @vitest-environment jsdom
import { afterEach, describe, expect, it } from "vitest";
import { claimHandshake, deviceWarning, handshakeFromLocation, type SessionState } from "./session";

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

/**
 * The desktop shell hands the app its address in the query string, and that query string used to
 * break every filter in the library: hash history writes the router's search after the `#` and
 * reads it from `window.location.search` before the `#`, so `#/library?tag=weekly` on a page whose
 * URL also carried `?token=…` parsed as `tag=weekly?token=…` and matched nothing.
 */
describe("claiming the handshake out of the url", () => {
  const go = (url: string) => window.history.replaceState(null, "", url);

  afterEach(() => {
    window.sessionStorage.clear();
    delete (globalThis as { __SUMMO__?: unknown }).__SUMMO__;
    go("/");
  });

  it("erases the query string and still knows where the daemon is", () => {
    go("/?port=8710&token=abc#/library?tag=weekly");
    claimHandshake();

    expect(window.location.search).toBe("");
    expect(window.location.hash).toBe("#/library?tag=weekly");
    expect(handshakeFromLocation(window.location.search)).toEqual({ port: 8710, token: "abc" });
  });

  // The URL is what a reload has, and after this runs the URL no longer says it.
  it("survives a reload, when the URL no longer carries it", () => {
    go("/?port=8710&token=abc");
    claimHandshake();
    go("/");

    expect(handshakeFromLocation("")).toEqual({ port: 8710, token: "abc" });
  });

  it("does nothing when the daemon served the page itself", () => {
    go("/#/library?tag=weekly");
    claimHandshake();
    expect(window.sessionStorage.length).toBe(0);
    expect(window.location.hash).toBe("#/library?tag=weekly");
  });

  /**
   * The ordinary shape when the daemon served the page: it injected the handshake, and a `token=`
   * left in the URL is a leftover that only does harm. It breaks the router exactly as badly as a
   * complete handshake would, so completeness is not the test — presence is.
   */
  it("removes a lone token, which is no handshake but breaks routing all the same", () => {
    (globalThis as { __SUMMO__?: unknown }).__SUMMO__ = { port: 7809, token: "injected" };
    go("/?token=abc#/library?tag=weekly");
    claimHandshake();

    expect(window.location.search).toBe("");
    expect(window.location.hash).toBe("#/library?tag=weekly");
    expect(handshakeFromLocation("")).toEqual({ port: 7809, token: "injected" });
  });

  it("keeps the rest of a query string it had to take two keys out of", () => {
    go("/?utm_source=somewhere&port=8710&token=abc#/library");
    claimHandshake();
    expect(window.location.search).toBe("?utm_source=somewhere");
  });

  it("leaves a query string that is not a handshake alone", () => {
    go("/?utm_source=somewhere#/library");
    claimHandshake();
    expect(window.location.search).toBe("?utm_source=somewhere");
  });

  it("is safe to call twice", () => {
    go("/?port=8710&token=abc#/library");
    claimHandshake();
    claimHandshake();
    expect(handshakeFromLocation("")).toEqual({ port: 8710, token: "abc" });
  });
});
