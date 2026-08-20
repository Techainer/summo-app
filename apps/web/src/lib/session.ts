/**
 * The recording session, as the app runs it.
 *
 * Ties the three moving parts together — the microphone, the daemon connection and the transcript —
 * so that `App` only has to say start and stop.
 *
 * The ordering in `start` matters and is easy to get wrong: the session is announced to the daemon
 * *before* the microphone opens, so that the first frame captured has somewhere to go. Opening the
 * microphone first would drop the beginning of whatever the user says while the daemon is still
 * loading a model — which is exactly the moment they are testing whether the app works.
 */

import { EngineClient, type ConnectionState, type Handshake } from "./engine";
import { Microphone, explainMicrophoneError } from "./microphone";
import type { Failure } from "./errors";
import type { Event, SessionSpec } from "./protocol";
import { url } from "./library";

export interface SessionCallbacks {
  onEvent: (event: Event) => void;
  onState: (state: SessionState) => void;
  onLevel?: (rms: number) => void;
}

export interface SessionState {
  recording: boolean;
  connection: ConnectionState;
  /**
   * Set when something went wrong that the user has to act on.
   *
   * A `Failure` rather than a sentence: the shell translates it, and a refused microphone needs a
   * different banner from a daemon that is not answering — one of them can be fixed from Settings
   * and the other cannot.
   */
  error: Failure | null;
  deviceLabel: string | null;
  /** The device's real rate. Below 16 kHz means a headset in telephony mode. */
  sampleRate: number | null;
  /**
   * The document this recording is writing into.
   *
   * A meeting is a note with a transcript in it, and this is which note. Without it the app knew a
   * recording was running and could not say into what, so recording had a screen of its own with a
   * transcript on it and nowhere to type — the one thing somebody in a meeting most wants to do.
   *
   * `null` while starting, and on a build with no recognition, where there is genuinely no file.
   */
  meeting: string | null;
}

export const NARROWBAND_HZ = 16000;

export class Session {
  private client: EngineClient | null = null;
  private microphone: Microphone | null = null;
  /** A refusal from the daemon, whenever it arrived. Cleared when a session starts. */
  private refused: { code: string; error: string } | null = null;
  /** True from the first press until the daemon is recording or has refused. */
  private starting = false;
  private state: SessionState = {
    recording: false,
    connection: "closed",
    error: null,
    deviceLabel: null,
    sampleRate: null,
    meeting: null,
  };

  constructor(
    private readonly handshake: Handshake,
    private readonly callbacks: SessionCallbacks,
  ) {}

  get current(): SessionState {
    return this.state;
  }

  private update(patch: Partial<SessionState>): void {
    this.state = { ...this.state, ...patch };
    this.callbacks.onState(this.state);
  }

  async start(spec: SessionSpec): Promise<void> {
    // Already recording, or already on the way there. The second guard is the one that was
    // missing: `start` is async — a socket to open, a model to load, a microphone to be granted —
    // and a second press during that window sent a second `session_start`. The daemon answered
    // "a recording is already in progress", in English, and the refusal tore down the session that
    // was working perfectly. Pressing a button twice must not stop a meeting.
    if (this.state.recording || this.starting) return;
    this.starting = true;
    this.refused = null;
    this.update({ error: null });

    this.client = new EngineClient(this.handshake, {
      onEvent: (event) => {
        // A refusal from the daemon ends the recording *here* too. Without this the app kept its
        // timer running, its button red and its banner up while the daemon sat idle — and the
        // failure was only visible to somebody who read the transcript that never appeared. A
        // transient error is different: the pipeline is still running and will catch up.
        if (event.kind === "error" && !event.transient) {
          // Remembered even when the refusal arrives before the microphone is open, which is the
          // usual case: `session_start` goes out, the daemon fails to load a model or finds no
          // voice detector, and the answer comes back while this code is still asking the browser
          // for a microphone. The old guard was `this.state.recording`, so that answer was dropped
          // on the floor — and a moment later the app set `recording: true` and ran a timer over a
          // session the daemon had already refused. Somebody watched it count to seventeen seconds
          // with nothing being recorded at all.
          this.refused = { code: event.code ?? "session_refused", error: event.message };
          if (this.state.recording) this.abandon(this.refused);
        }
        this.callbacks.onEvent(event);
      },
      onState: (connection) => this.update({ connection }),
    });
    this.client.connect();

    // Wait for the socket before announcing the session; a command sent while connecting would be
    // dropped and the daemon would never start recording.
    const connected = await this.waitForOpen(5000);
    if (!connected) {
      this.update({
        error: {
          code: "engine_silent",
          error: "The recognition engine is not responding. Is summo-engine running?",
        },
      });
      this.client.close();
      this.client = null;
      this.starting = false;
      return;
    }

    this.client.send({ cmd: "session_start", ...spec });

    this.microphone = new Microphone({
      onFrame: (samples) => this.client?.sendAudio("mic", samples),
      onLevel: this.callbacks.onLevel,
    });

    try {
      await this.microphone.start();
    } catch (error) {
      this.update({ error: explainMicrophoneError(error) });
      this.client.send({ cmd: "session_stop" });
      this.client.close();
      this.client = null;
      this.microphone = null;
      this.starting = false;
      return;
    }

    // The daemon may already have said no while the microphone was opening. Checked here rather
    // than trusted to arrive later: this is the last moment before the app starts claiming to
    // record.
    if (this.refused) {
      this.abandon(this.refused);
      return;
    }
    this.starting = false;

    this.update({
      recording: true,
      deviceLabel: this.microphone.deviceLabel,
      sampleRate: this.microphone.sampleRate,
    });

    // Which document this is going into, asked of the daemon rather than parsed out of the log
    // line it prints. Not awaited: recording has already started, and the page it opens is a
    // convenience — a session that records perfectly while this request is in flight is a session
    // that recorded perfectly.
    void this.findMeeting();
  }

  /**
   * Put everything down: the microphone, the socket, the claim to be recording.
   *
   * One path for "the daemon will not do this", whether it says so in the first fifty milliseconds
   * or the fiftieth second.
   */
  private abandon(refusal: { code: string; error: string }): void {
    this.starting = false;
    this.microphone?.stop();
    this.microphone = null;
    this.client?.close();
    this.client = null;
    this.update({ recording: false, error: refusal });
  }

  /**
   * Ask what the daemon is writing into, briefly.
   *
   * The id is minted before the session begins, so the first answer is usually the right one. It is
   * polled anyway because `start` returns as soon as the microphone is open, and on a slow machine
   * the daemon may still be loading a model — three attempts over a second and a half, then give
   * up, because a missing id costs the live view and nothing else.
   */
  private async findMeeting(): Promise<void> {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      if (!this.state.recording) return;
      try {
        const response = await fetch(url(this.handshake, "/status"));
        if (response.ok) {
          const body = (await response.json()) as { meeting?: string };
          if (body.meeting) {
            this.update({ meeting: body.meeting });
            return;
          }
        }
      } catch {
        // The status endpoint being briefly unreachable is not worth a banner: the socket is the
        // thing carrying the meeting, and it is up.
      }
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }

  /**
   * Change what is listening, without ending the meeting.
   *
   * A meeting is not always in the language the settings say, and that is discovered *during* it —
   * usually in the first sentence. Stopping and starting again costs exactly the part where
   * somebody noticed, so this leaves the recording, the file and everything transcribed alone and
   * rebuilds only the decoder.
   */
  retune(language: string): void {
    if (!this.state.recording) return;
    this.client?.send({ cmd: "model_swap", language });
  }

  stop(): void {
    this.microphone?.stop();
    this.microphone = null;

    // Stop the daemon's session before closing the socket, so it flushes the open utterance and
    // writes the file rather than treating the disconnect as an abandoned recording.
    this.client?.send({ cmd: "session_stop" });
    // The meeting stays: the page the user is looking at is the one that was just recorded, and
    // clearing it here would close the document out from under them at the moment they want to
    // read it.
    this.update({ recording: false });

    // Give the daemon a moment to answer with the saved path before the socket goes.
    const client = this.client;
    this.client = null;
    setTimeout(() => client?.close(), 2000);
  }

  /**
   * What the user has typed into the meeting so far.
   *
   * The whole section every time, because this is a save rather than a diff — the app owns the
   * text and the daemon owns the file. Silently ignored when nothing is recording: the editor is
   * the same editor a finished note uses, and it saves through the notes API once the meeting has
   * ended.
   */
  notes(text: string): void {
    if (!this.state.recording) return;
    this.client?.send({ cmd: "notes", text });
  }

  /** Audio frames buffered because the socket is down. Surfaced in the status bar. */
  get buffered(): number {
    return this.client?.buffered ?? 0;
  }

  private waitForOpen(timeoutMs: number): Promise<boolean> {
    return new Promise((resolve) => {
      const started = Date.now();
      const poll = () => {
        if (this.client?.state === "open") return resolve(true);
        if (Date.now() - started > timeoutMs) return resolve(false);
        setTimeout(poll, 50);
      };
      poll();
    });
  }
}

/**
 * A warning about the capture device, if it deserves one.
 *
 * The case worth catching is a Bluetooth headset: using one for input switches the link to a
 * telephony profile at 8 or 16 kHz, recognition falls apart, and to the user the app simply looks
 * broken — the same headset sounded fine playing music a minute earlier.
 */
export function deviceWarning(state: SessionState): string | null {
  if (state.sampleRate !== null && state.sampleRate < NARROWBAND_HZ) {
    return `This microphone is running at ${Math.round(state.sampleRate / 1000)} kHz. Bluetooth headsets switch to a low-quality mode when recording — a built-in or USB microphone will be noticeably more accurate.`;
  }
  const label = state.deviceLabel?.toLowerCase() ?? "";
  if (/airpods|bluetooth|hands-free|headset|buds|jabra|beats/.test(label)) {
    return "Bluetooth headsets switch to a low-quality telephony mode when recording. A built-in or USB microphone will be noticeably more accurate.";
  }
  return null;
}

/** Where a handshake taken out of the URL is kept, so a reload can still find it. */
const STASH = "summo.handshake";

/**
 * Move the handshake out of the query string, before anything reads the URL.
 *
 * The desktop shell loads the app as `…?port=8710&token=…`, which is the only way to hand a
 * webview an address it cannot have injected into. That query string then breaks the router: hash
 * history writes the router's own search *after* the `#` and reads it from `window.location.search`
 * *before* the `#`, so it parses `#/library?tag=weekly` on a page whose URL also has `?token=…` as
 * `tag=weekly?token=…`. Every filter in the app came back empty, and only in the desktop build.
 *
 * So the handshake is claimed once and the query string erased. `sessionStorage` rather than a
 * variable because the URL is what a reload has, and after this runs the URL no longer says it —
 * losing the daemon's address on refresh would be a worse bug than the one being fixed. It is also
 * what the token wanted anyway: a token in a URL ends up in history, in the window title, and in
 * whatever a user pastes into a bug report.
 *
 * Call before the router is created. It is safe to call twice and does nothing when the daemon
 * served the page itself, since then there was never a query string to begin with.
 */
export function claimHandshake(): void {
  if (typeof window === "undefined" || window.location.search === "") return;
  const params = new URLSearchParams(window.location.search);
  if (!params.has("port") && !params.has("token")) return;

  // Whether or not the two make a usable handshake. A lone `token=` breaks the router exactly as
  // badly as a complete one and is the ordinary shape when the daemon served the page — it has
  // already injected what it needs, and what is left in the URL is a leftover that only does harm.
  const found = readHandshake({
    port: Number(params.get("port")),
    token: params.get("token"),
  });
  if (found) {
    try {
      window.sessionStorage.setItem(STASH, JSON.stringify(found));
    } catch {
      // Private mode, or storage disabled. Keep the URL, since it is now the only copy: a broken
      // filter is a smaller failure than an app that cannot reach its daemon.
      return;
    }
  }

  // Only these two keys. Anything else in the query belongs to whoever put it there.
  params.delete("port");
  params.delete("token");
  const rest = params.toString();
  window.history.replaceState(
    window.history.state,
    "",
    `${window.location.pathname}${rest === "" ? "" : `?${rest}`}${window.location.hash}`,
  );
}

/** Read the daemon's address from the URL, for development and for the desktop shell. */
export function handshakeFromLocation(search: string): Handshake | null {
  // Injected by the daemon when it serves the page itself — see `summo_engine::assets`. Preferred
  // over the query string, because a token in a URL lands in history, in the window title and in
  // whatever the user pastes into a bug report. The query string stays for the Tauri webview,
  // which is loaded from a custom scheme and has nowhere to inject into.
  const injected = (globalThis as { __SUMMO__?: unknown }).__SUMMO__;
  const fromInjection = readHandshake(injected);
  if (fromInjection) return fromInjection;

  // Whatever `claimHandshake` took out of the URL, including across a reload.
  const stashed = readStash();
  if (stashed) return stashed;

  const params = new URLSearchParams(search);
  return readHandshake({
    port: Number(params.get("port")),
    token: params.get("token"),
  });
}

function readStash(): Handshake | null {
  try {
    const raw = window.sessionStorage.getItem(STASH);
    return raw === null ? null : readHandshake(JSON.parse(raw));
  } catch {
    // Unparseable or unavailable: fall through to the query string rather than fail to connect.
    return null;
  }
}

/** Accept a handshake only if both halves are usable; half of one is not a connection. */
function readHandshake(value: unknown): Handshake | null {
  if (typeof value !== "object" || value === null) return null;
  const { port, token } = value as { port?: unknown; token?: unknown };
  if (typeof port !== "number" || !Number.isInteger(port) || port <= 0) return null;
  if (typeof token !== "string" || token.length === 0) return null;
  return { port, token };
}
