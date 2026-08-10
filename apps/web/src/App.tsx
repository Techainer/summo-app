import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Transcript } from "./components/Transcript";
import { RecordButton } from "./components/RecordButton";
import { StatusBar } from "./components/StatusBar";
import { Waveform } from "./components/Waveform";
import { Library } from "./components/Library";
import { LibraryClient } from "./lib/library";
import { Settings } from "./components/Settings";
import { apply, empty, type TranscriptState } from "./lib/transcript";
import { Session, deviceWarning, handshakeFromLocation, type SessionState } from "./lib/session";
import type { Event } from "./lib/protocol";

/** Where the daemon is, when the app was not launched by the shell. */
const DEV_HANDSHAKE = { port: 8710, token: "" };

const IDLE: SessionState = {
  recording: false,
  connection: "closed",
  error: null,
  deviceLabel: null,
  sampleRate: null,
};

/**
 * The main window.
 *
 * Deliberately thin. Transcript ordering lives in `lib/transcript`, capture and connection in
 * `lib/session`, and both are tested without a DOM; what is left here is layout and the one piece of
 * behaviour that belongs to the window — that recording keeps going when the window is not focused.
 */
export function App() {
  const [transcript, setTranscript] = useState<TranscriptState>(empty);
  const [session, setSession] = useState<SessionState>(IDLE);
  const [elapsed, setElapsed] = useState(0);
  const [level, setLevel] = useState(0);
  const [stat, setStat] = useState<{ rtf: number; rss_mb: number; queue_ms: number } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [compact, setCompact] = useState(false);
  const [screen, setScreen] = useState<"record" | "library" | "settings">("record");

  const timer = useRef<number | null>(null);
  const controller = useRef<Session | null>(null);

  const onEvent = useCallback((event: Event) => {
    switch (event.kind) {
      case "stat":
        setStat({ rtf: event.rtf, rss_mb: event.rss_mb, queue_ms: event.queue_ms });
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

  if (controller.current === null) {
    controller.current = new Session(handshake, {
      onEvent,
      onState: setSession,
      onLevel: setLevel,
    });
  }

  const start = useCallback(async () => {
    setElapsed(0);
    setScreen("record");
    timer.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
    await controller.current?.start({ live_model: "gipformer-65m", lanes: ["mic"] });
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

  const speakers = useMemo(() => {
    const seen = new Set<string>();
    for (const s of transcript.segments) if (s.speaker) seen.add(s.speaker);
    return [...seen];
  }, [transcript.segments]);

  const warning = deviceWarning(session);
  const latest = transcript.segments.at(-1);

  if (compact) {
    return (
      <div className="mini" data-testid="compact">
        <RecordButton recording={session.recording} elapsed={elapsed} onToggle={toggle} />
        <Waveform level={level} active={session.recording} />
        <p className="mini-text">{latest?.text ?? "Đang nghe…"}</p>
        <button
          type="button"
          className="icon-button"
          onClick={() => setCompact(false)}
          aria-label="Mở rộng cửa sổ"
          title="Mở rộng"
        >
          ⤢
        </button>
      </div>
    );
  }

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          <span className="bars" aria-hidden>
            <i /><i /><i />
          </span>
          Summo
        </div>
        <nav className="tabs" aria-label="Màn hình">
          <button
            type="button"
            className={screen === "record" ? "on" : ""}
            onClick={() => setScreen("record")}
          >
            Ghi
          </button>
          <button
            type="button"
            className={screen === "library" ? "on" : ""}
            onClick={() => setScreen("library")}
          >
            Thư viện
          </button>
          <button
            type="button"
            className={screen === "settings" ? "on" : ""}
            onClick={() => setScreen("settings")}
          >
            Cài đặt
          </button>
        </nav>
        <div className="header-actions">
          <Waveform level={level} active={session.recording} />
          <RecordButton recording={session.recording} elapsed={elapsed} onToggle={toggle} />
          <button
            type="button"
            className="icon-button"
            onClick={() => setCompact(true)}
            aria-label="Thu gọn cửa sổ"
            title="Thu gọn khi đang họp"
          >
            ⤡
          </button>
        </div>
      </header>

      {session.error && <div className="banner error">{session.error}</div>}
      {warning && <div className="banner warn">{warning}</div>}

      <main className="app-main">
        {screen === "settings" ? (
          <Settings handshake={handshake} />
        ) : screen === "library" ? (
          <Library client={library} onRecord={() => void start()} />
        ) : transcript.segments.length === 0 ? (
          <p className="empty">
            {session.recording ? "Đang nghe…" : "Bấm ghi để bắt đầu. Mọi thứ chạy trên máy bạn."}
          </p>
        ) : (
          <Transcript segments={transcript.segments} />
        )}
      </main>

      <StatusBar
        stat={stat}
        speakers={speakers}
        notice={notice}
        connection={session.connection}
        device={session.deviceLabel}
      />
    </div>
  );
}
