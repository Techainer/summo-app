import { useCallback, useEffect, useImperativeHandle, useRef, useState, type Ref } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
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
  const [error, setError] = useState<string | null>(null);

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

  const onTimeUpdate = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    setTime(element.currentTime);
    onTime?.(element.currentTime);
  }, [onTime]);

  const toggle = useCallback(() => {
    const element = audio.current;
    if (!element) return;
    if (element.paused) void element.play().catch((e: unknown) => setError(String(e)));
    else element.pause();
  }, []);

  if (!current) {
    return (
      <p className="border-line bg-bg-soft text-fg-faint rounded-[var(--radius-card)] border px-4 py-3 text-[13px]">
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
        onError={() => setError(t("meeting.cannot_open_audio"))}
      >
        <track kind="captions" />
      </audio>

      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={toggle}
          aria-label={playing ? t("meeting.pause") : t("meeting.play")}
          className="bg-accent text-accent-fg grid size-9 shrink-0 place-items-center rounded-full hover:brightness-110"
        >
          <span aria-hidden="true">{playing ? "⏸" : "▶"}</span>
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

        <span className="tabular text-fg-dim shrink-0 text-[12px]">
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
        <div className="ml-auto flex items-center gap-1">
          {SPEEDS.map((option) => (
            <button
              key={option}
              type="button"
              onClick={() => setSpeed(option)}
              aria-pressed={speed === option}
              className={cn(
                "tabular rounded-full px-2 py-1 text-[12px] transition-colors",
                speed === option
                  ? "bg-accent-soft text-accent font-medium"
                  : "text-fg-faint hover:text-fg",
              )}
            >
              {option}×
            </button>
          ))}
        </div>
      </div>

      {error && <p className="text-rec mt-2 text-[12px]">{error}</p>}
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

/** `h:mm:ss` once past an hour, `m:ss` before it — a 40-minute meeting should not read `0:40:12`. */
export function clock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "0:00";
  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
