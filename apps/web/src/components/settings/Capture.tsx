import { useCallback, useMemo, useState } from "react";

import { Button, Checkbox } from "../ui";
import { HINT, LABEL } from "./fields";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";

/**
 * The numbers a recording is actually made with.
 *
 * All of these have been in `settings.toml` since the daemon was written, enforced on every
 * session, and reachable only by editing that file — which means the two most consequential
 * decisions in the product were made once, by us, for everybody:
 *
 * **How much silence ends a sentence.** It is added directly to the delay before final text
 * appears. A person who finds the transcript slow is feeling this number and has no way to say so.
 *
 * **How loud counts as speech.** Too high in a quiet room and half of what was said never becomes
 * an utterance at all; too low in a café and the keyboard is transcribed.
 *
 * Sliders rather than boxes, with the shipped value marked, because both are judgements about a
 * room rather than quantities anybody knows in milliseconds.
 */

interface RecordingSettings {
  capture_system_audio: boolean;
  device_id: string | null;
  hotkey: string;
  suggest_on_meeting: boolean;
  vad_threshold: number;
  min_silence_ms: number;
  threads: number | null;
}

/** What the daemon ships with, drawn under each slider so a change can be undone by eye. */
const SHIPPED = { vad_threshold: 0.5, min_silence_ms: 500 };

export function Capture() {
  const t = useT();
  const { handshake } = useEngine();
  const say = useErrorText();
  const [error, setError] = useState<string | null>(null);

  const settings = useLoad(
    useCallback(async () => {
      const response = await fetch(url(handshake, "/settings"));
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const body = (await response.json()) as {
        settings?: { recording?: Partial<RecordingSettings>; models?: { threads?: number | null } };
      };
      return {
        capture_system_audio: body.settings?.recording?.capture_system_audio ?? false,
        device_id: body.settings?.recording?.device_id ?? null,
        hotkey: body.settings?.recording?.hotkey ?? "",
        suggest_on_meeting: body.settings?.recording?.suggest_on_meeting ?? true,
        vad_threshold: body.settings?.recording?.vad_threshold ?? SHIPPED.vad_threshold,
        min_silence_ms: body.settings?.recording?.min_silence_ms ?? SHIPPED.min_silence_ms,
        threads: body.settings?.models?.threads ?? null,
      } satisfies RecordingSettings;
    }, [handshake]),
    [handshake],
  );

  // The value being dragged, so a slider moves under the finger rather than after the round trip.
  const [live, setLive] = useState<Partial<RecordingSettings>>({});
  const now = useMemo(
    () => ({ ...(settings.data ?? ({} as RecordingSettings)), ...live }),
    [settings.data, live],
  );

  const write = async (patch: Partial<RecordingSettings>) => {
    try {
      const response = await fetch(url(handshake, "/settings/recording"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(patch),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      setError(null);
      settings.reload();
    } catch (e) {
      setError(say(e));
    }
  };

  if (!settings.data) {
    return <p className="text-fg-faint text-meta">{settings.error ?? t("common.loading")}</p>;
  }

  return (
    <section
      data-testid="settings-capture"
      className="border-line bg-bg-raised mt-6 rounded-2xl border p-5"
    >
      <h3 className="font-medium">{t("settings.capture_heading")}</h3>
      <p className="text-fg-dim text-meta mt-1 mb-4">{t("settings.capture_hint")}</p>

      <Checkbox
        checked={now.capture_system_audio ?? false}
        onChange={(on) => {
          setLive((current) => ({ ...current, capture_system_audio: on }));
          void write({ capture_system_audio: on });
        }}
      >
        {t("settings.capture_system")}
      </Checkbox>
      <p className={HINT}>{t("settings.capture_system_hint")}</p>

      <Checkbox
        className="mt-3.5"
        checked={now.suggest_on_meeting ?? true}
        onChange={(on) => {
          setLive((current) => ({ ...current, suggest_on_meeting: on }));
          void write({ suggest_on_meeting: on });
        }}
      >
        {t("settings.suggest_on_meeting")}
      </Checkbox>
      <p className={HINT}>{t("settings.suggest_on_meeting_hint")}</p>

      {/* The two that decide how a sentence is cut. */}
      <label className="mt-5 block">
        <span className={LABEL}>
          {t("settings.silence", { ms: String(now.min_silence_ms ?? SHIPPED.min_silence_ms) })}
        </span>
        <input
          type="range"
          min={120}
          max={2000}
          step={20}
          value={now.min_silence_ms ?? SHIPPED.min_silence_ms}
          aria-label={t("settings.silence_label")}
          data-testid="min-silence"
          onChange={(event) =>
            setLive((current) => ({ ...current, min_silence_ms: Number(event.target.value) }))
          }
          onPointerUp={() => void write({ min_silence_ms: now.min_silence_ms })}
          onBlur={() => void write({ min_silence_ms: now.min_silence_ms })}
          className="accent-accent mt-1 w-full"
        />
        <span className={HINT}>{t("settings.silence_hint")}</span>
      </label>

      <label className="mt-4 block">
        <span className={LABEL}>
          {t("settings.threshold", {
            value: (now.vad_threshold ?? SHIPPED.vad_threshold).toFixed(2),
          })}
        </span>
        <input
          type="range"
          min={0.05}
          max={0.95}
          step={0.05}
          value={now.vad_threshold ?? SHIPPED.vad_threshold}
          aria-label={t("settings.threshold_label")}
          data-testid="vad-threshold"
          onChange={(event) =>
            setLive((current) => ({ ...current, vad_threshold: Number(event.target.value) }))
          }
          onPointerUp={() => void write({ vad_threshold: now.vad_threshold })}
          onBlur={() => void write({ vad_threshold: now.vad_threshold })}
          className="accent-accent mt-1 w-full"
        />
        <span className={HINT}>{t("settings.threshold_hint")}</span>
      </label>

      {/* Threads. Zero means "follow the hardware probe", which is what a fresh install does and
          what anybody should leave it at until a recording is competing with a build. */}
      <label className="mt-4 block">
        <span className={LABEL}>{t("settings.threads")}</span>
        <input
          type="number"
          min={0}
          max={64}
          value={now.threads ?? 0}
          aria-label={t("settings.threads")}
          data-testid="threads"
          onChange={(event) =>
            setLive((current) => ({ ...current, threads: Number(event.target.value) }))
          }
          onBlur={() => void write({ threads: now.threads ?? 0 })}
          className="border-line bg-bg-soft text-fg h-9 w-24 rounded-[var(--radius-card)] border px-2 text-sm"
        />
        <span className={HINT}>{t("settings.threads_hint")}</span>
      </label>

      <div className="mt-5 flex items-center gap-3">
        {/* Not `busy`: a busy button is a disabled one, and the click that lands on this button
            has just blurred a slider — which starts a save. The reset was being swallowed by the
            write it followed. */}
        <Button
          size="sm"
          variant="secondary"
          onClick={() =>
            void write({
              vad_threshold: SHIPPED.vad_threshold,
              min_silence_ms: SHIPPED.min_silence_ms,
              threads: 0,
            }).then(() => setLive({}))
          }
        >
          {t("settings.reset_capture")}
        </Button>
        {error && <span className="text-rec text-micro">{error}</span>}
      </div>
    </section>
  );
}
