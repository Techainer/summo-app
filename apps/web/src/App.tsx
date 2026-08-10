import { useCallback, useMemo, useRef, useState } from "react";
import { Transcript } from "./components/Transcript";
import { RecordButton } from "./components/RecordButton";
import { StatusBar } from "./components/StatusBar";
import { apply, empty, type TranscriptState } from "./lib/transcript";
import type { Event } from "./lib/protocol";

/**
 * The main window.
 *
 * Deliberately thin: transcript state lives in a plain reducer in `lib/transcript`, which is where
 * the ordering rules are tested. Anything here is layout.
 */
export function App() {
  const [transcript, setTranscript] = useState<TranscriptState>(empty);
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [stat, setStat] = useState<{ rtf: number; rss_mb: number; queue_ms: number } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const timer = useRef<number | null>(null);

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

  const toggle = useCallback(() => {
    setRecording((was) => {
      if (was) {
        if (timer.current !== null) window.clearInterval(timer.current);
        timer.current = null;
        return false;
      }
      setElapsed(0);
      timer.current = window.setInterval(() => setElapsed((e) => e + 1), 1000);
      return true;
    });
  }, []);

  const speakers = useMemo(() => {
    const seen = new Set<string>();
    for (const s of transcript.segments) if (s.speaker) seen.add(s.speaker);
    return [...seen];
  }, [transcript.segments]);

  return (
    <div className="app">
      <header className="app-header">
        <div className="brand">
          <span className="bars" aria-hidden>
            <i /><i /><i />
          </span>
          Summo
        </div>
        <RecordButton recording={recording} elapsed={elapsed} onToggle={toggle} />
      </header>

      <main className="app-main">
        {transcript.segments.length === 0 ? (
          <p className="empty">
            {recording
              ? "Đang nghe…"
              : "Bấm ghi để bắt đầu. Mọi thứ chạy trên máy bạn."}
          </p>
        ) : (
          <Transcript
            segments={transcript.segments}
            onEvent={onEvent}
          />
        )}
      </main>

      <StatusBar stat={stat} speakers={speakers} notice={notice} />
    </div>
  );
}
