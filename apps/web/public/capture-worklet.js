/**
 * Microphone capture, on the audio thread.
 *
 * The browser hands audio to this processor in 128-sample blocks at whatever rate the device runs
 * at — 44.1 or 48 kHz, rarely 16. Everything downstream of capture in Summo speaks 16 kHz mono, so
 * the conversion happens here, once, before anything crosses back to the main thread.
 *
 * Two constraints shape the code. This runs on a real-time thread, so it must not allocate per
 * block or do anything unbounded; and `postMessage` costs a copy, so blocks are accumulated into
 * 100 ms frames rather than posted 375 times a second.
 */

const TARGET_RATE = 16000;
const FRAME_SAMPLES = 1600; // 100 ms at 16 kHz

class CaptureProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this.frame = new Float32Array(FRAME_SAMPLES);
    this.filled = 0;
    // Fractional read position into the input block, carried across blocks so resampling does not
    // restart its phase every 128 samples and add a periodic artefact.
    this.position = 0;
    this.ratio = sampleRate / TARGET_RATE;
  }

  process(inputs) {
    const channels = inputs[0];
    if (!channels || channels.length === 0) return true;

    const first = channels[0];
    if (!first || first.length === 0) return true;

    // Average channels: a stereo interface often carries the useful signal on one side only, and
    // taking channel zero would lose 3 dB for no reason.
    const blockLength = first.length;
    const mono = channels.length === 1 ? first : this.downmix(channels, blockLength);

    while (this.position < blockLength) {
      const index = Math.floor(this.position);
      const fraction = this.position - index;
      const a = mono[index] ?? 0;
      const b = mono[index + 1] ?? a;

      // Linear interpolation. Good enough for speech at these rates, and cheap enough for the
      // audio thread; the alternative is a windowed sinc that would cost more than the VAD.
      this.frame[this.filled++] = a + (b - a) * fraction;

      if (this.filled === FRAME_SAMPLES) {
        // Transferred rather than copied, so the main thread takes ownership of the buffer.
        const out = this.frame.slice();
        this.port.postMessage(out, [out.buffer]);
        this.filled = 0;
      }
      this.position += this.ratio;
    }
    this.position -= blockLength;
    return true;
  }

  downmix(channels, length) {
    const mono = new Float32Array(length);
    for (const channel of channels) {
      for (let i = 0; i < length; i += 1) mono[i] += channel[i] ?? 0;
    }
    const scale = 1 / channels.length;
    for (let i = 0; i < length; i += 1) mono[i] *= scale;
    return mono;
  }
}

registerProcessor("summo-capture", CaptureProcessor);
