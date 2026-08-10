/**
 * Opening the microphone and delivering 16 kHz frames.
 *
 * The conversion to Summo's format happens on the audio thread (see `public/capture-worklet.js`);
 * this is the part that has to deal with permission, with devices that lie about what they are, and
 * with a browser that will happily "help" in ways that hurt recognition.
 */

export interface MicrophoneOptions {
  /** Called with each 100 ms frame of 16 kHz mono audio. */
  onFrame: (samples: Float32Array) => void;
  /** Called with a signal level in 0..1, for the waveform. */
  onLevel?: (rms: number) => void;
  deviceId?: string;
}

export class Microphone {
  private context: AudioContext | null = null;
  private stream: MediaStream | null = null;
  private node: AudioWorkletNode | null = null;

  constructor(private readonly options: MicrophoneOptions) {}

  /**
   * Ask for the microphone and start delivering frames.
   *
   * Noise suppression is turned **off** on purpose. The browser's suppressor is tuned for making a
   * call sound pleasant, and it does that by removing anything it decides is not speech — which on
   * a quiet speaker eats the start of words and leaves recognition guessing. Echo cancellation and
   * gain control stay on: both help, and neither invents or deletes signal the way suppression does.
   */
  async start(): Promise<void> {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        deviceId: this.options.deviceId ? { exact: this.options.deviceId } : undefined,
        echoCancellation: true,
        autoGainControl: true,
        noiseSuppression: false,
        channelCount: 1,
      },
    });

    // Let the context run at the device's own rate and resample in the worklet. Forcing 16 kHz here
    // makes some browsers resample with a worse algorithm than ours, and others refuse outright.
    this.context = new AudioContext();
    await this.context.audioWorklet.addModule("capture-worklet.js");

    const source = this.context.createMediaStreamSource(this.stream);
    this.node = new AudioWorkletNode(this.context, "summo-capture");

    this.node.port.onmessage = (message) => {
      const samples = message.data as Float32Array;
      this.options.onFrame(samples);
      if (this.options.onLevel) this.options.onLevel(rms(samples));
    };

    source.connect(this.node);
    // The worklet produces no output, but some browsers suspend a graph with no path to the
    // destination. A zero-gain node keeps it alive without anything being audible.
    const silence = this.context.createGain();
    silence.gain.value = 0;
    this.node.connect(silence).connect(this.context.destination);
  }

  stop(): void {
    this.node?.port.close();
    this.node?.disconnect();
    this.stream?.getTracks().forEach((track) => track.stop());
    void this.context?.close();
    this.node = null;
    this.stream = null;
    this.context = null;
  }

  /** The device actually opened, which may not be the one that was asked for. */
  get deviceLabel(): string | null {
    return this.stream?.getAudioTracks()[0]?.label ?? null;
  }

  /**
   * The rate the device is really running at.
   *
   * Worth surfacing: a Bluetooth headset in its telephony mode reports 8 or 16 kHz here, and that
   * is the single largest quality cliff in the whole pipeline.
   */
  get sampleRate(): number | null {
    return this.context?.sampleRate ?? null;
  }
}

/** Root-mean-square level of a frame. */
export function rms(samples: Float32Array): number {
  if (samples.length === 0) return 0;
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length);
}

/** Turn a permission failure into something a person can act on. */
export function explainMicrophoneError(error: unknown): string {
  const name = error instanceof DOMException ? error.name : "";
  switch (name) {
    case "NotAllowedError":
      return "Microphone access was refused. Grant it in your browser or system settings and try again.";
    case "NotFoundError":
      return "No microphone was found. Plug one in, or check that it is enabled in system settings.";
    case "NotReadableError":
      return "The microphone is in use by another application.";
    case "OverconstrainedError":
      return "That microphone is no longer available. Pick another one in settings.";
    default:
      return error instanceof Error ? error.message : "The microphone could not be opened.";
  }
}
