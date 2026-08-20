/**
 * The engine provider, apart from `useEngine`.
 *
 * Same reason as `i18n/provider.tsx`: a module that exports a component *and* a hook cannot be
 * hot-replaced, so editing the daemon plumbing used to remount the whole app and lose whatever
 * screen state was on the screen at the time.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { DEV_HANDSHAKE, EngineContext, IDLE, type EngineValue, type Stat } from "./engine-context";
import type { Handshake } from "./engine";
import type { Failure } from "./errors";
import { LibraryClient } from "./library";
import { PeopleClient } from "./people";
import type { Event } from "./protocol";
import { load as loadCapture, save as saveCapture } from "./capture";
import { Session, handshakeFromLocation, type SessionState } from "./session";
import { bridgeShellEvents, inShell, shellHandshake } from "./shell";
import { apply, empty, type TranscriptState } from "./transcript";
import { Starting } from "../components/shell/Starting";

export function EngineProvider({ children }: { children: ReactNode }) {
  const [transcript, setTranscript] = useState<TranscriptState>(empty);
  const [session, setSession] = useState<SessionState>(IDLE);
  const [elapsed, setElapsed] = useState(0);
  const [level, setLevel] = useState(0);
  const [stat, setStat] = useState<Stat | null>(null);
  // Kept as a `Failure`, not a sentence. This provider sits *outside* the one that holds the
  // language — translating here called `useI18n` where there is none — and the raw shape is the
  // right thing to keep anyway: state holds what happened, the screen decides how to say it.
  const [notice, setNotice] = useState<Failure | null>(null);

  const timer = useRef<number | null>(null);

  const onEvent = useCallback((event: Event) => {
    switch (event.kind) {
      case "stat":
        setStat({
          rtf: event.rtf,
          rss_mb: event.rss_mb,
          queue_ms: event.queue_ms,
        });
        break;
      case "error":
        // With its code, so the status bar can say it in the reader's language. This used to put
        // the daemon's own sentence on screen — `session needs a live model`, in English, under a
        // Vietnamese interface, the first time anybody pressed record without a model.
        setNotice({ error: event.message, ...(event.code ? { code: event.code } : {}) });
        break;
      case "info":
        // Not shown. These are the daemon's own log lines — "session started with gipformer-65m —
        // writing to /tmp/…/2026-08-20-hop-03-57.md" — and they were being printed along the
        // bottom of the window, filesystem path and all, to somebody who wanted to know whether
        // their meeting was being recorded. The red button and the clock say that.
        break;
        break;
      default:
        setTranscript((state) => apply(state, event));
    }
  }, []);

  /**
   * Where the daemon is.
   *
   * Three sources, and which one applies is decided once. A page the daemon served carries the
   * handshake in the document; a page the desktop or mobile shell loaded from its bundle has to
   * ask the shell for it, which takes as long as the engine takes to start; a page opened by
   * `pnpm dev` has neither and talks to whatever is on the development port.
   *
   * `null` means "asking" — the app renders nothing but a splash until it has an answer, because
   * every screen in it is a view of the vault and there is no vault to view yet.
   */
  const [handshake, setHandshake] = useState<Handshake | null>(
    () => handshakeFromLocation(window.location.search) ?? (inShell() ? null : DEV_HANDSHAKE),
  );
  const [shellError, setShellError] = useState<string | null>(null);

  useEffect(() => {
    if (handshake !== null) return undefined;
    let live = true;
    void shellHandshake().then(
      (found) => {
        if (live) setHandshake(found);
      },
      (error: unknown) => {
        if (live) setShellError(error instanceof Error ? error.message : String(error));
      },
    );
    return () => {
      live = false;
    };
  }, [handshake]);

  // The tray icon and the global shortcut, translated into the DOM event the record button already
  // listens for. Once, for the life of the app.
  useEffect(() => {
    let stop: (() => void) | null = null;
    let live = true;
    void bridgeShellEvents().then((found) => {
      if (live) stop = found;
      else found?.();
    });
    return () => {
      live = false;
      stop?.();
    };
  }, []);

  // `DEV_HANDSHAKE` only until the real one lands: hooks cannot be skipped, and nothing fetches
  // through these clients before the splash below has been replaced by the app.
  const library = useMemo(() => new LibraryClient(handshake ?? DEV_HANDSHAKE), [handshake]);
  const people = useMemo(() => new PeopleClient(handshake ?? DEV_HANDSHAKE), [handshake]);

  /**
   * The recording session, rebuilt only if the daemon's address changes — which happens once, when
   * a shell answers with the real one, and never while anything is being recorded.
   *
   * A `useMemo` rather than the lazily-filled ref this used to be. The ref was fine while the
   * handshake was decided before the first render; comparing one ref against another to decide
   * whether to rebuild is the pattern React's own lint rule exists to stop.
   */
  const controller = useMemo(
    () =>
      handshake === null
        ? null
        : new Session(handshake, { onEvent, onState: setSession, onLevel: setLevel }),
    [handshake, onEvent],
  );

  const start = useCallback(async () => {
    setElapsed(0);
    setTranscript(empty);
    // Read at the moment of starting, not captured at mount: the tray icon and the global shortcut
    // both reach `toggle` without going through the screen, and they have to honour whatever the
    // user last chose rather than whatever was set when the window opened.
    const chosen = loadCapture();
    await controller?.start({
      // Deliberately not named here. This was hardcoded to `gipformer-65m`, which made the model
      // catalogue decorative: installing a Japanese model changed nothing, because recording still
      // reached for the Vietnamese transducer. The daemon reads the setting, and falls back to the
      // only installed speech model when there is one.
      live_model: "",
      lanes: chosen.lanes,
      // The spoken language, when the user chose one. Empty is not "unset" here — it is Whisper's
      // own detection — so it is only sent when it has a value, and the daemon falls back to
      // `models.language` from the settings file otherwise.
      ...(chosen.spoken ? { language: chosen.spoken } : {}),
      // Diarization needs the system lane; asking for it on the microphone alone is refused.
      diarize: chosen.lanes.includes("system"),
      ...(chosen.translateTo ? { translate_to: chosen.translateTo } : {}),
    });
  }, [controller]);

  // Mid-meeting, and it also updates what the *next* meeting starts from: somebody who corrects
  // the language during a call has told us the setting was wrong, not only this recording.
  const retune = useCallback(
    (language: string) => {
      const current = loadCapture();
      saveCapture({ ...current, spoken: language });
      controller?.retune(language);
    },
    [controller],
  );

  // Debounced by the editor, not here: this is the save, and the daemon writes the file the moment
  // it arrives.
  const notes = useCallback(
    (text: string) => {
      controller?.notes(text);
    },
    [controller],
  );

  const stop = useCallback(() => {
    setLevel(0);
    controller?.stop();
  }, [controller]);

  /**
   * The clock runs while the daemon is recording, and not one second before.
   *
   * It used to start the moment the button was pressed. When the daemon refused the session — no
   * voice detector installed, for instance — nothing stopped it, so the app showed a running timer,
   * a red button and a level meter over a recording that did not exist. A user watched it reach
   * seventeen seconds and asked, fairly, what it thought it was doing.
   */
  useEffect(() => {
    if (!session.recording) {
      if (timer.current !== null) window.clearInterval(timer.current);
      timer.current = null;
      return undefined;
    }
    timer.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    return () => {
      if (timer.current !== null) window.clearInterval(timer.current);
      timer.current = null;
    };
  }, [session.recording]);

  const toggle = useCallback(() => {
    if (session.recording) stop();
    else void start();
  }, [session.recording, start, stop]);

  // The tray and the global shortcut both reach the window this way, so recording can start
  // without the app being focused — the whole point of having a shortcut at all.
  useEffect(() => {
    const onExternalToggle = () => toggle();
    window.addEventListener("summo:toggle-record", onExternalToggle);
    return () => window.removeEventListener("summo:toggle-record", onExternalToggle);
  }, [toggle]);

  // A recording that stops because someone tidied their desktop would be maddening, so leaving the
  // page is confirmed rather than silently accepted.
  useEffect(() => {
    if (!session.recording) return undefined;
    const warn = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener("beforeunload", warn);
    return () => window.removeEventListener("beforeunload", warn);
  }, [session.recording]);

  const value = useMemo<EngineValue>(
    () => ({
      library,
      people,
      handshake: handshake ?? DEV_HANDSHAKE,
      session,
      transcript,
      elapsed,
      level,
      stat,
      notice,
      dismissNotice: () => setNotice(null),
      start,
      stop,
      toggle,
      retune,
      notes,
    }),
    [
      library,
      people,
      handshake,
      session,
      transcript,
      elapsed,
      level,
      stat,
      notice,
      start,
      stop,
      toggle,
      retune,
      notes,
    ],
  );

  // After every hook, never before one. The app is not rendered at all while the shell is still
  // starting its engine: a screen that fetches from a port nobody is on would fill with failures
  // the user cannot act on and would have to watch disappear a second later.
  if (handshake === null) return <Starting error={shellError} />;

  return <EngineContext.Provider value={value}>{children}</EngineContext.Provider>;
}
