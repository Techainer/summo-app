/**
 * The client side of the daemon connection.
 *
 * Two things here are less obvious than they look.
 *
 * **Reconnection is not optional.** The daemon runs in its own process precisely so a crashed
 * inference kernel does not take the meeting with it — but that only pays off if the app comes back
 * automatically and keeps the transcript it already has.
 *
 * **Audio keeps flowing while disconnected.** Frames captured during a reconnect are buffered, up to
 * a bound. Dropping them silently would leave a gap in the recording that nothing later can recover.
 */

import { decodeDub, type DubChunk } from "./listen";
import { encodeFrame, type Command, type Event, type Lane } from "./protocol";

/** How long to buffer audio while reconnecting, in frames of 100 ms. */
const MAX_BUFFERED_FRAMES = 300; // 30 seconds

/** Reconnect backoff, milliseconds. Caps quickly: this is loopback, not the internet. */
const BACKOFF_MS = [100, 250, 500, 1000, 2000] as const;

export interface Handshake {
  port: number;
  token: string;
}

export type ConnectionState = "connecting" | "open" | "reconnecting" | "closed";

export interface EngineOptions {
  onEvent: (event: Event) => void;
  onState?: (state: ConnectionState) => void;
  /**
   * A piece of dubbed audio, when somebody asked to hear the translation.
   *
   * Separate from `onEvent` because it is not one: an event is JSON that ends up in the transcript,
   * and this is sound that must never go near it. The recording is what was said.
   */
  onDub?: (chunk: DubChunk) => void;
}

export class EngineClient {
  private socket: WebSocket | null = null;
  private pending: ArrayBuffer[] = [];
  private attempt = 0;
  private closedByUs = false;
  private droppedFrames = 0;

  constructor(
    private readonly handshake: Handshake,
    private readonly options: EngineOptions,
  ) {}

  get state(): ConnectionState {
    if (this.closedByUs) return "closed";
    if (!this.socket) return "connecting";
    return this.socket.readyState === WebSocket.OPEN
      ? "open"
      : this.attempt > 0
        ? "reconnecting"
        : "connecting";
  }

  /** Frames discarded because a reconnect outlasted the buffer. Surfaced in the HUD. */
  get dropped(): number {
    return this.droppedFrames;
  }

  connect(): void {
    this.closedByUs = false;
    const { port, token } = this.handshake;
    // The token goes in the query because a browser WebSocket cannot set headers. The daemon's
    // origin check is what stops a web page from using it.
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws?token=${encodeURIComponent(token)}`);
    socket.binaryType = "arraybuffer";
    this.socket = socket;
    this.options.onState?.(this.attempt > 0 ? "reconnecting" : "connecting");

    socket.onopen = () => {
      this.attempt = 0;
      this.options.onState?.("open");
      this.flush();
    };

    socket.onmessage = (message) => {
      // Binary is dubbed audio. This branch used to be a bare `return` — everything non-string on
      // the floor — which was right while nothing sent any, and is the hook the live dub plugs into.
      if (message.data instanceof ArrayBuffer) {
        const chunk = decodeDub(message.data);
        if (chunk) this.options.onDub?.(chunk);
        return;
      }
      if (typeof message.data !== "string") return;
      try {
        this.options.onEvent(JSON.parse(message.data) as Event);
      } catch (e) {
        // A malformed frame is the daemon's bug, not a reason to tear down the session — but it
        // was also not a reason to say nothing. Dropped in silence, the symptom is a feature that
        // simply does not happen, with nothing anywhere connecting it to a frame that arrived
        // broken. The daemon's own tests treat this as a fault in the other direction:
        // `protocol::tests::an_empty_frame_is_an_error_not_a_silent_no_op`.
        //
        // `warn` rather than `error`, in development only: `shots.mjs` fails a screen on a console
        // error, and a daemon bug should not be reported as a broken screen. A user gets the same
        // resilience as before and none of the noise.
        if (import.meta.env.DEV) {
          console.warn("the daemon sent a frame this build could not read", e, message.data);
        }
      }
    };

    socket.onclose = () => {
      if (this.closedByUs) return;
      this.scheduleReconnect();
    };

    socket.onerror = () => socket.close();
  }

  private scheduleReconnect(): void {
    const delay = BACKOFF_MS[Math.min(this.attempt, BACKOFF_MS.length - 1)] ?? 2000;
    this.attempt += 1;
    this.options.onState?.("reconnecting");
    setTimeout(() => {
      if (!this.closedByUs) this.connect();
    }, delay);
  }

  send(command: Command): void {
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(JSON.stringify(command));
    }
  }

  /** Queue a captured frame, buffering it if the socket is down. */
  sendAudio(lane: Lane, samples: Float32Array): void {
    const frame = encodeFrame(lane, samples);
    if (this.socket?.readyState === WebSocket.OPEN) {
      this.socket.send(frame);
      return;
    }
    this.pending.push(frame);
    while (this.pending.length > MAX_BUFFERED_FRAMES) {
      this.pending.shift();
      this.droppedFrames += 1;
    }
  }

  private flush(): void {
    if (this.socket?.readyState !== WebSocket.OPEN) return;
    for (const frame of this.pending) this.socket.send(frame);
    this.pending = [];
  }

  get buffered(): number {
    return this.pending.length;
  }

  close(): void {
    this.closedByUs = true;
    this.socket?.close();
    this.socket = null;
    this.options.onState?.("closed");
  }
}
