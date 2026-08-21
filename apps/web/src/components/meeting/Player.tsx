import { Pause, Play } from "lucide-react";
import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";

import { useT } from "../../i18n/context";
import { clock } from "../../lib/clock";
import { SegmentedControl } from "../ui";

export interface PlayerHandle {
  /** Move the playhead, in seconds. Used when a transcript line is clicked. */
  seek: (seconds: number) => void;
}

interface Props {
  /** Absolute URLs, one per lane, already carrying the daemon token. */
  lanes: { key: string; label: string; url: string }[];
  /** Seconds at which each utterance starts, drawn as marks on the scrubber. */
  marks?: number[];
  /** Reported as playback moves, so the transcript can follow along. */
  onTime?: (seconds: number) => void;
  ref?: Ref<PlayerHandle>;
}

const SPEEDS = [0.75, 1, 1.25, 1.5, 2];

/**
 * Listening back to a meeting.
 *
 * The scrubber carries a mark per utterance, which is the cheap version of the chapter dots in the
 * reference design and more useful here: the gaps between marks are the silences, so the shape of
 * the bar shows at a glance where the conversation actually was.
 *
 * Speed is a control rather than a preference because the reason to use it changes within one
 * recording — 2× through a status round, 1× through the part that mattered.
 */
export function Player({ lanes, marks = [], onTime, ref }: Props) {
  const t = useT();
  const audio = useRef<HTMLAudioElement>(null);
  const [lane, setLane] = useState(lanes[0]?.key ?? "");
  const [playing, setPlaying] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [speed, setSpeed] = useState(1);
  /**
   * The lane that would not play, and why.
   *
   * Stored *with* its lane rather than as a bare flag, so switching lanes clears it without an
   * effect having to remember to: a different lane is a different file, and one of them failing
   * says nothing about the next.
   *
   * It was `setError(String(e))`, which put a raw DOMException in front of the reader, drawn in
   * `text-rec` — the recording red — under a transport stuck at `0:00 / 0:00`. That is an alarm,
   * and this is not an alarm. The transcript is complete either way; what has gone wrong is that
   * one file cannot be opened, which is worth saying once, quietly, in the reader's own language.
   *
   * The first attempt at this replaced the whole transport with the "audio was pruned" notice.
   * That was wrong twice over, and `e2e/meeting.mjs` caught it: it takes away the lane picker and
   * the retry, and it asserts a *reason* — pruning — that nobody established. A file that will not
   * open was not necessarily pruned; it may have been moved, or be mid-write, or be corrupt.
   * Saying which would be inventing one.
   */
  const [failed, setFailed] = useState<{ lane: string } | null>(null);

  const current = lanes.find((l) => l.key === lane) ?? lanes[0];

  useImperativeHandle(ref, () => ({
    seek(seconds) {
      const element = audio.current;
      if (!element) return;
      element.currentTime = seconds;
      setTime(seconds);
      // Seeking from the transcript means "play this bit", so start if we were paused.
      void element.play().catch(() => undefined);
    },
  }));

  useEffect(() => {
    const element = audio.current;
    if (!element) return;
    element.playbackRate = speed;
  }, [speed, lane]);

  const unplayable = failed?.lane === lane;

  const onTimeUpdate = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    setTime(element.currentTime);
    onTime?.(element.currentTime);
  }, [onTime]);

  const toggle = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    if (element.paused) void element.play().catch(() => setFailed({ lane }));
    else element.pause();
  }, [lane]);

  // No lane at all: audio retention is off and the daemon kept none. This one *is* the pruned case
  // — there is no file to fail — so it is the one place the pruned sentence is true.
  if (!current) {
    return (
      <p className="border-line bg-bg-soft text-fg-faint text-meta rounded-[var(--radius-card)] border px-4 py-3">
        {t("meeting.no_audio")}
      </p>
    );
  }

  return (
    <div className="border-line bg-bg-raised rounded-[var(--radius-card)] border p-3">
      <audio
        ref={audio}
        src={current.url}
        preload="metadata"
        onLoadedMetadata={(e) => setDuration(e.currentTarget.duration || 0)}
        onTimeUpdate={onTimeUpdate}
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onError={() => setFailed({ lane })}
      >
        <track kind="captions" />
      </audio>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={toggle}
          aria-label={playing ? t("meeting.pause") : t("meeting.play")}
          className="bg-accent text-accent-fg grid size-9 shrink-0 place-items-center rounded-full transition-all duration-150 hover:brightness-110 active:scale-95"
        >
          {/* Filled, and the play triangle nudged right by a pixel. A triangle centred on its
              bounding box looks left of centre inside a circle, because its mass is not where its
              box is — every media player in the world corrects for this. */}
          {playing ? (
            <Pause aria-hidden="true" className="size-4 fill-current" />
          ) : (
            <Play aria-hidden="true" className="ms-px size-4 fill-current" />
          )}
        </button>

        <Scrubber
          time={time}
          duration={duration}
          marks={marks}
          onSeek={(seconds) => {
            const element = audio.current;
            if (!element) return;
            element.currentTime = seconds;
            setTime(seconds);
          }}
        />

        <span className="tabular text-fg-dim text-micro shrink-0">
          {clock(time)} / {clock(duration)}
        </span>
      </div>

      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        {lanes.length > 1 && (
          <SegmentedControl
            label={t("record.audio_source")}
            size="sm"
            options={lanes.map((l) => ({ value: l.key, label: l.label }))}
            value={lane}
            onChange={setLane}
          />
        )}
        {/* The same control as the lane picker beside it, rather than a row of loose pills that
            happened to look like one. Five bare buttons with their own hover colours read as five
            unrelated links; a segmented control reads as one choice with five positions, which is
            what it is. Values are strings because that is what a segmented control switches on —
            `speed` stays a number, since it is what `playbackRate` wants. */}
        <SegmentedControl
          className="ml-auto"
          label={t("meeting.speed")}
          size="sm"
          options={SPEEDS.map((option) => ({ value: String(option), label: `${option}×` }))}
          value={String(speed)}
          onChange={(next) => setSpeed(Number(next))}
        />
      </div>

      {/* Said once, quietly, and without taking the controls away. `fg-dim` rather than `rec`:
          this is information, not an alarm — the transcript above it is complete, and the lane
          picker beside it is how somebody tries the other take. */}
      {unplayable && (
        <p className="text-fg-dim text-micro mt-2">{t("meeting.cannot_open_audio")}</p>
      )}
    </div>
  );
}

function Scrubber({
  time,
  duration,
  marks,
  onSeek,
}: {
  time: number;
  duration: number;
  marks: number[];
  onSeek: (seconds: number) => void;
}) {
  const t = useT();
  const percent = duration > 0 ? (time / duration) * 100 : 0;

  return (
    <div className="relative flex-1">
      {/* A range input rather than a div with a click handler: it comes with keyboard support,
          which a scrubber genuinely needs — arrow keys are how you find an exact moment. */}
      <input
        type="range"
        min={0}
        max={Math.max(duration, 0.1)}
        step={0.1}
        value={time}
        onChange={(e) => onSeek(Number(e.target.value))}
        aria-label={t("meeting.seek")}
        aria-valuetext={clock(time)}
        className="peer [&::-webkit-slider-thumb]:bg-accent relative z-10 h-6 w-full cursor-pointer appearance-none bg-transparent [&::-webkit-slider-thumb]:size-3 [&::-webkit-slider-thumb]:appearance-none [&::-webkit-slider-thumb]:rounded-full"
      />
      <span
        aria-hidden="true"
        className="bg-bg-soft pointer-events-none absolute inset-x-0 top-1/2 h-1 -translate-y-1/2 rounded-full"
      >
        <span className="bg-accent block h-full rounded-full" style={{ width: `${percent}%` }} />
      </span>
      {/* Where somebody was speaking. The gaps are the silences. */}
      {duration > 0 &&
        marks.map((at, i) => (
          <span
            key={`${at}-${i}`}
            aria-hidden="true"
            className="bg-fg-faint/50 pointer-events-none absolute top-1/2 h-2 w-px -translate-y-1/2"
            style={{ left: `${Math.min((at / duration) * 100, 100)}%` }}
          />
        ))}
    </div>
  );
}
