import { describe, expect, it } from "vitest";
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
