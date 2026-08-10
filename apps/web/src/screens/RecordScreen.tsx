import { useEngine } from "../lib/engine-context";
import { Transcript } from "../components/Transcript";

/** What the app shows while it is listening, and the invitation to start when it is not. */
export function RecordScreen() {
  const { transcript, session } = useEngine();

  if (transcript.segments.length === 0) {
    return (
      <p className="mt-24 text-center text-fg-faint">
        {session.recording ? "Đang nghe…" : "Bấm ghi để bắt đầu. Mọi thứ chạy trên máy bạn."}
      </p>
    );
  }
  return <Transcript segments={transcript.segments} />;
}
