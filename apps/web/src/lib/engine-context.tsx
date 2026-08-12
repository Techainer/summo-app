import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { LibraryClient } from "./library";
import { PeopleClient } from "./people";
import type { Event } from "./protocol";
import { load as loadCapture } from "./capture";
import { Session, handshakeFromLocation, type SessionState } from "./session";
import { apply, empty, type TranscriptState } from "./transcript";

/** Where the daemon is, when the app was not launched by the shell. */
const DEV_HANDSHAKE = { port: 8710, token: "" };

const IDLE: SessionState = {
  recording: false,
  connection: "closed",
  error: null,
  deviceLabel: null,
  sampleRate: null,
};

export interface Stat {
  rtf: number;
  rss_mb: number;
  queue_ms: number;
}

interface EngineValue {
  library: LibraryClient;
  people: PeopleClient;
  handshake: { port: number; token: string };
  session: SessionState;
  transcript: TranscriptState;
  elapsed: number;
  level: number;
  stat: Stat | null;
  notice: string | null;
  dismissNotice: () => void;
  start: () => Promise<void>;
  stop: () => void;
  toggle: () => void;
}

const EngineContext = createContext<EngineValue | null>(null);

/**
 * The live connection to the daemon, held above the router.
 *
 * Recording has to survive navigation: opening the library mid-meeting must not tear down the
 * WebSocket. Keeping the session here rather than inside a screen is what makes that true — a
 * screen can unmount and the recording continues.
 */
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

export function useEngine(): EngineValue {
  const value = useContext(EngineContext);
  if (!value) throw new Error("useEngine must be used inside an EngineProvider");
  return value;
}
