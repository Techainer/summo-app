import { describe, expect, it } from "vitest";

import { TAG_DUB, decodeDub } from "./listen";

/**
 * The daemon's encoder is the other half of this and it has its own round-trip test. What these
 * check is that this half agrees with the format the daemon writes — the two are hand-written and
 * the failure when they drift is silence, which looks exactly like a feature that is switched off.
 */

/** Build the bytes `summo_engine::livedub::Chunk::encode` produces. */
function encode(seq: number, lang: string, rate: number, samples: number[]): ArrayBuffer {
  const tag = new TextEncoder().encode(lang);
  const buffer = new ArrayBuffer(2 + tag.length + 12 + samples.length * 4);
  const view = new DataView(buffer);
  view.setUint8(0, TAG_DUB);
  view.setUint8(1, tag.length);
  new Uint8Array(buffer, 2, tag.length).set(tag);
  view.setUint32(2 + tag.length, rate, true);
  view.setBigUint64(2 + tag.length + 4, BigInt(seq), true);
  samples.forEach((s, i) => view.setFloat32(2 + tag.length + 12 + i * 4, s, true));
  return buffer;
}

describe("decodeDub", () => {
  it("reads back what the daemon writes", () => {
    const chunk = decodeDub(encode(7, "vi", 22050, [-1, 0, 0.5, 1]));
    expect(chunk).not.toBeNull();
    expect(chunk?.seq).toBe(7);
    expect(chunk?.lang).toBe("vi");
    expect(chunk?.rate).toBe(22050);
    expect([...(chunk?.samples ?? [])]).toEqual([-1, 0, 0.5, 1]);
  });

  /** Sequence numbers are 64-bit on the wire, and a `getUint32` here would wrap silently. */
  it("survives a sequence number past what fits in 32 bits", () => {
    expect(decodeDub(encode(4_294_967_296, "en", 16000, [0]))?.seq).toBe(4_294_967_296);
  });

  /** The socket may carry other binary frames. Not-mine is not an error. */
  it("returns null rather than throwing on anything that is not a dub", () => {
    expect(decodeDub(new ArrayBuffer(0))).toBeNull();
    expect(decodeDub(new ArrayBuffer(4))).toBeNull();
    const wrongTag = encode(1, "vi", 16000, [0]);
    new DataView(wrongTag).setUint8(0, 0x02);
    expect(decodeDub(wrongTag)).toBeNull();
  });

  /** A body that is not a whole number of samples is corrupt, not short. */
  it("refuses a truncated body", () => {
    const whole = encode(1, "vi", 16000, [0.5]);
    expect(decodeDub(whole.slice(0, whole.byteLength - 1))).toBeNull();
  });

  /**
   * The samples must not alias the socket's buffer: a view over it changes when the next frame
   * reuses the memory, and audio that changes after it was queued is a bug nobody can reproduce.
   */
  it("copies the samples out of the frame", () => {
    const buffer = encode(1, "vi", 16000, [0.25]);
    const chunk = decodeDub(buffer);
    new DataView(buffer).setFloat32(buffer.byteLength - 4, 0.99, true);
    expect(chunk?.samples[0]).toBe(0.25);
  });
});
