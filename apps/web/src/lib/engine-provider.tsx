/**
 * The engine provider, apart from `useEngine`.
 *
 * Same reason as `i18n/provider.tsx`: a module that exports a component *and* a hook cannot be
 * hot-replaced, so editing the daemon plumbing used to remount the whole app and lose whatever
 * screen state was on the screen at the time.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { DEV_HANDSHAKE, EngineContext, IDLE, type EngineValue, type Stat } from "./engine-context";
import { LibraryClient } from "./library";
import { PeopleClient } from "./people";
import type { Event } from "./protocol";
import { load as loadCapture } from "./capture";
import { Session, handshakeFromLocation, type SessionState } from "./session";
import { apply, empty, type TranscriptState } from "./transcript";

export function EngineProvider({ children }: { children: ReactNode }) {
  const [transcript, setTranscript] = useState<TranscriptState>(empty);
  const [session, setSession] = useState<SessionState>(IDLE);
  const [elapsed, setElapsed] = useState(0);
  const [level, setLevel] = useState(0);
  const [stat, setStat] = useState<Stat | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const timer = useRef<number | null>(null);
  const controller = useRef<Session | null>(null);

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
        setNotice(event.message);
        break;
      case "info":
        setNotice(event.text);
        break;
      default:
        setTranscript((state) => apply(state, event));
    }
  }, []);

  const handshake = useMemo(
    () => handshakeFromLocation(window.location.search) ?? DEV_HANDSHAKE,
    [],
  );
  const library = useMemo(() => new LibraryClient(handshake), [handshake]);
  const people = useMemo(() => new PeopleClient(handshake), [handshake]);

  if (controller.current === null) {
    controller.current = new Session(handshake, {
      onEvent,
      onState: setSession,
      onLevel: setLevel,
    });
  }

  const start = useCallback(async () => {
    setElapsed(0);
    setTranscript(empty);
    timer.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    // Read at the moment of starting, not captured at mount: the tray icon and the global shortcut
    // both reach `toggle` without going through the screen, and they have to honour whatever the
    // user last chose rather than whatever was set when the window opened.
    const chosen = loadCapture();
    await controller.current?.start({
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
  }, []);

  const stop = useCallback(() => {
    if (timer.current !== null) window.clearInterval(timer.current);
    timer.current = null;
    setLevel(0);
    controller.current?.stop();
  }, []);

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
      handshake,
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
    ],
  );

  return <EngineContext.Provider value={value}>{children}</EngineContext.Provider>;
}
