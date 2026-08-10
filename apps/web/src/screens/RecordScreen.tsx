import { useState } from "react";

import { Transcript } from "../components/Transcript";
import { ImportPanel } from "../components/import/ImportPanel";
import { SegmentedControl } from "../components/ui";
import { useEngine } from "../lib/engine-context";

type Source = "record" | "upload";

/**
 * What the app shows while it is listening, and the two ways to start.
 *
 * The `[Ghi | Nhập file]` control is the entry point from the LetFlow reference, and it is the
 * first thing that tells a new user the app takes files at all — an import buried in a menu is an
 * import nobody finds.
 *
 * Switching to upload while recording is deliberately allowed: the recording keeps running in the
 * daemon, and forbidding it would mean a user who wants to queue a file has to stop their meeting
 * first. The transcript comes back the moment they switch back.
 */
export function RecordScreen() {
  const { transcript, session } = useEngine();
  const [source, setSource] = useState<Source>("record");

  return (
    <div className="flex h-full flex-col">
      <div className="flex justify-center px-4 pt-4">
        <SegmentedControl
          label="Nguồn"
          size="sm"
          value={source}
          onChange={setSource}
          options={[
            { value: "record", label: "Ghi" },
            { value: "upload", label: "Nhập file" },
          ]}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {source === "upload" ? (
          <ImportPanel />
        ) : transcript.segments.length === 0 ? (
          <p className="mt-24 text-center text-fg-faint">
            {session.recording ? "Đang nghe…" : "Bấm ghi để bắt đầu. Mọi thứ chạy trên máy bạn."}
          </p>
        ) : (
          <Transcript segments={transcript.segments} />
        )}
      </div>
    </div>
  );
}
