/**
 * Hearing the meeting in another language.
 *
 * The daemon synthesises the translation and sends it down the same socket the transcript rides,
 * as binary frames. This is the other half of `summo_engine::livedub::Chunk::encode` and the queue
 * that plays what it decodes.
 *
 * ## Why the sound comes out here and not in the daemon
 *
 * The daemon could open an output device. It must not, for one reason that decides it: this app
 * captures system audio. A dub played through the speakers is captured, transcribed, translated and
 * spoken again — a loop, in a feature whose whole point is listening to a meeting. Playing it in the
 * client keeps the audio on a device the listener chose, which in practice is headphones.
 *
 * It also gets something for free that the other way could not: the dub is per *listener*. Two
 * people on one recording can hear two different languages, because each one's browser plays its
 * own.
 */

/** First byte of a dub frame. Mirrors `summo_engine::livedub::TAG_DUB`. */
export const TAG_DUB = 0x01;

export interface DubChunk {
  /** The utterance this speaks, so it can be lined up with the transcript. */
  seq: number;
  lang: string;
  rate: number;
  samples: Float32Array;
}

/**
 * Read one frame, or `null` if it is not a dub.
 *
 * `null` rather than a throw: the socket is allowed to carry other binary frames later, and a
 * decoder that treats "not mine" as an error turns every future addition into a console full of
 * failures.
 */
export function decodeDub(buffer: ArrayBuffer): DubChunk | null {
  if (buffer.byteLength < 14) return null;
  const view = new DataView(buffer);
  if (view.getUint8(0) !== TAG_DUB) return null;

  const langLength = view.getUint8(1);
  const head = 2 + langLength;
  if (buffer.byteLength < head + 12) return null;

  const lang = new TextDecoder().decode(new Uint8Array(buffer, 2, langLength));
  const rate = view.getUint32(head, true);
  // `getBigUint64` because a sequence number is 64 bits on the wire. Back to a number afterwards:
  // sequence numbers count utterances, so the exact integer range runs out somewhere past a
  // thousand years of continuous speech.
  const seq = Number(view.getBigUint64(head + 4, true));

  const body = buffer.byteLength - (head + 12);
  if (body < 0 || body % 4 !== 0) return null;
  // Copied rather than viewed. A `Float32Array` over the socket's buffer would alias memory the
  // next frame reuses, and the bug that causes is audio that changes after it was queued.
  const samples = new Float32Array(buffer.slice(head + 12));
  return { seq, lang, rate, samples };
}

/**
 * Plays dubbed chunks back to back, on a device and at a volume the listener chose.
 *
 * Scheduled against the audio clock rather than played on arrival. `AudioBufferSourceNode.start`
 * takes a time, and giving it one is the difference between speech and speech with gaps in it: the
 * timers a browser gives JavaScript are not accurate enough to butt two buffers together, and every
 * inaccuracy would be an audible click.
 */
export class DubPlayer {
  private context: AudioContext | null = null;
  private gain: GainNode | null = null;
  /** When the audio already scheduled will have finished, on the context's own clock. */
  private until = 0;
  private volume = 1;

  /** How much sound is scheduled and not yet played, in seconds. For the interface to show. */
  get backlogSeconds(): number {
    if (!this.context) return 0;
    return Math.max(0, this.until - this.context.currentTime);
  }

  async start(deviceId?: string): Promise<void> {
    if (this.context) return;
    this.context = new AudioContext();
    this.gain = this.context.createGain();
    this.gain.gain.value = this.volume;
    this.gain.connect(this.context.destination);
    this.until = this.context.currentTime;
    await this.setDevice(deviceId);
  }

  /**
   * Send the dub to a chosen output.
   *
   * Best effort on purpose. `setSinkId` is not everywhere, and a browser without it should play the
   * dub on the default device rather than refuse to play it — losing the feature over the choice of
   * device would be the larger failure.
   */
  async setDevice(deviceId?: string): Promise<void> {
    // `setSinkId` on an `AudioContext` is newer than the type definitions here, so the capability
    // is asked for rather than assumed — which is also how it should be treated at runtime.
    const context: (AudioContext & { setSinkId?: (id: string) => Promise<void> }) | null =
      this.context;
    if (!deviceId || !context?.setSinkId) return;
    try {
      await context.setSinkId(deviceId);
    } catch {
      // The device was unplugged, or the browser will not allow it. Either way: keep playing.
    }
  }

  setVolume(volume: number): void {
    this.volume = Math.min(1, Math.max(0, volume));
    if (this.gain && this.context) {
      // Ramped, not assigned. A gain that jumps produces a click, which on a volume slider means
      // every drag is a burst of them.
      this.gain.gain.setTargetAtTime(this.volume, this.context.currentTime, 0.02);
    }
  }

  /** Queue a chunk. Returns the delay before it will be heard, in seconds. */
  play(chunk: DubChunk): number {
    if (!this.context || !this.gain || chunk.samples.length === 0) return 0;

    const buffer = this.context.createBuffer(1, chunk.samples.length, chunk.rate);
    // A fresh array rather than the decoded one: `copyToChannel` wants a `Float32Array` over a
    // plain `ArrayBuffer`, and what came off the socket is typed over `ArrayBufferLike`.
    buffer.copyToChannel(new Float32Array(chunk.samples), 0);
    const source = this.context.createBufferSource();
    source.buffer = buffer;
    source.connect(this.gain);

    // From whichever is later: now, or the end of what is already queued. Taking `currentTime`
    // alone would start every chunk immediately and play them over each other.
    const at = Math.max(this.context.currentTime, this.until);
    source.start(at);
    this.until = at + buffer.duration;
    return at - this.context.currentTime;
  }

  /**
   * Stop everything and release the device.
   *
   * Closing rather than suspending: an `AudioContext` left open holds the output device, and on a
   * laptop that is the difference between the speakers idling and the fans not stopping.
   */
  async stop(): Promise<void> {
    const context = this.context;
    this.context = null;
    this.gain = null;
    this.until = 0;
    if (context) await context.close().catch(() => undefined);
  }
}
