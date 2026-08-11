import { useState } from "react";

import { Transcript } from "../components/Transcript";
import { Waveform } from "../components/Waveform";
import { ImportPanel } from "../components/import/ImportPanel";
import { CaptureControls } from "../components/record/CaptureControls";
import { SegmentedControl } from "../components/ui";
import { useT } from "../i18n/context";
import { cn } from "../lib/cn";
import { useEngine } from "../lib/engine-context";
import { formatTime } from "../lib/protocol";

type Source = "record" | "upload";

/**
 * The screen before anything has been said.
 *
 * It used to be one grey sentence in the middle of an empty pane, with the only way to start a
 * recording a small button up in the header — so the screen whose entire job is recording had
 * nothing on it that recorded. This is the button, at the size of the thing it does.
 *
 * The header button stays: a meeting has to be stoppable from wherever the user has navigated to,
 * and this screen is not where they will be. Both drive the same `toggle`, so there is one
 * behaviour and two places to reach it, not two controls that can disagree.
 */
function Idle() {
  const { session, elapsed, level, toggle } = useEngine();
  const t = useT();

  return (
    <div className="mt-14 flex flex-col items-center gap-5 px-4 text-center">
      <button
        type="button"
        onClick={toggle}
        aria-pressed={session.recording}
        aria-label={session.recording ? t("record.stop") : t("record.start")}
        className={cn(
          "grid size-24 place-items-center rounded-full border-2 transition-colors",
          "focus:outline-none focus-visible:border-accent",
          session.recording
            ? "border-rec bg-rec-soft"
            : "border-line-strong bg-bg-soft hover:border-rec",
        )}
      >
        {/* A circle that becomes a square, which is what every recorder in the world does. The
            pulse is only while recording: a dot that always throbs stops meaning anything. */}
        <span
          aria-hidden="true"
          className={cn(
            "bg-rec transition-all duration-200",
            session.recording
              ? "size-7 rounded-[6px] motion-safe:animate-[pulse_1.6s_ease-in-out_infinite]"
              : "size-11 rounded-full",
          )}
        />
      </button>

      {session.recording ? (
        <>
          <p className="tabular text-2xl font-semibold">{formatTime(elapsed)}</p>
          <Waveform level={level} active />
          <p className="text-[13px] text-fg-faint">{t("record.listening")}</p>
        </>
      ) : (
        <p className="max-w-sm text-[13px] leading-relaxed text-fg-faint">{t("record.idle")}</p>
      )}
    </div>
  );
}

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
  const { transcript } = useEngine();
  const [source, setSource] = useState<Source>("record");
  const t = useT();

  return (
    <div className="flex h-full flex-col">
      <div className="flex justify-center px-4 pt-4">
        <SegmentedControl
          label={t("record.source")}
          size="sm"
          value={source}
          onChange={setSource}
          options={[
            { value: "record", label: t("record.tab_record") },
            { value: "upload", label: t("record.tab_upload") },
          ]}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-auto">
        {source === "upload" ? (
          <ImportPanel />
        ) : (
          <>
            <CaptureControls />
            {transcript.segments.length === 0 ? (
              <Idle />
            ) : (
              <Transcript segments={transcript.segments} />
            )}
          </>
        )}
      </div>
    </div>
  );
}
