import { useCallback, useMemo, useState } from "react";
import { load as loadCapture, save as saveCapture, setSystemAudio } from "../../lib/capture";

import { Button, Checkbox, Select } from "../ui";
import { HINT, LABEL } from "./fields";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { inputDevices } from "../../lib/permissions";

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
  suggest_on_meeting: boolean;
  hotkey: string;
  vad_threshold: number;
  min_silence_ms: number;
  threads: number | null;
}

/** What the daemon ships with, drawn under each slider so a change can be undone by eye. */
const SHIPPED = { vad_threshold: 0.5, min_silence_ms: 500 };

/**
 * The shortcut a fresh install has. Mirrors `summo_core::settings::Recording::default`.
 *
 * `CmdOrCtrl` is ⌘ on macOS and Ctrl everywhere else, which is why it is written that way rather
 * than resolved here — the desktop shell parses the same string and knows which platform it is on.
 */
const DEFAULT_HOTKEY = "CmdOrCtrl+Shift+R";

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
        suggest_on_meeting: body.settings?.recording?.suggest_on_meeting ?? true,
        hotkey: body.settings?.recording?.hotkey ?? DEFAULT_HOTKEY,
        vad_threshold: body.settings?.recording?.vad_threshold ?? SHIPPED.vad_threshold,
        min_silence_ms: body.settings?.recording?.min_silence_ms ?? SHIPPED.min_silence_ms,
        threads: body.settings?.models?.threads ?? null,
      } satisfies RecordingSettings;
    }, [handshake]),
    [handshake],
  );

  /**
   * The microphones this browser can see.
   *
   * Names are hidden until permission has been granted — that is the specification, not a quirk —
   * so before the user has said yes this is a list of anonymous ids, and the picker says so rather
   * than drawing five blank rows. A failure here costs the picker, not the screen.
   */
  const devices = useLoad(
    useCallback(async () => {
      try {
        return await inputDevices();
      } catch {
        return [];
      }
    }, []),
    [],
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

  /**
   * Save the shortcut, then make it live.
   *
   * Two steps because two processes own half the answer each: the daemon keeps the setting, and
   * the desktop shell holds the operating system's registration. Saving without the second is the
   * bug this setting had for its whole life — the file changed and the keystroke did not.
   *
   * The rebind is fire-and-forget and deliberately not an error here. Outside the desktop shell
   * there is nothing to rebind and nothing is wrong; inside it, a combination the OS refuses is
   * reported by the shell on its own terms, and a saved setting that could not be bound is still a
   * saved setting.
   */
  const writeHotkey = async (value: string) => {
    await write({ hotkey: value.trim() });
    const tauri = (globalThis as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    if (!tauri) return;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("set_hotkey");
    } catch {
      // The shell says so in its own log; a settings screen is not where an OS refusal belongs.
    }
  };

  if (!settings.data) {
    return <p className="text-fg-faint text-meta">{settings.error ?? t("common.loading")}</p>;
  }

  return (
    <section
      data-testid="settings-capture"
      className="border-line bg-bg-raised rounded-card mt-6 border p-5"
    >
      <h3 className="font-medium">{t("settings.capture_heading")}</h3>
      <p className="text-fg-dim text-meta mt-1 mb-4">{t("settings.capture_hint")}</p>

      {/* Both halves, because this one fact had two homes.
          The daemon's copy is what this screen reads back and what `/settings` reports; the lanes
          in `localStorage` are what a recording actually opens. Writing only the first made this
          switch a control that changed a number nothing consulted. */}
      <Checkbox
        checked={now.capture_system_audio ?? false}
        onChange={(on) => {
          setLive((current) => ({ ...current, capture_system_audio: on }));
          saveCapture(setSystemAudio(loadCapture(), on));
          void write({ capture_system_audio: on });
        }}
      >
        {t("settings.capture_system")}
      </Checkbox>
      <p className={HINT}>{t("settings.capture_system_hint")}</p>

      <Checkbox
        className="mt-4"
        checked={now.suggest_on_meeting ?? true}
        onChange={(on) => {
          setLive((current) => ({ ...current, suggest_on_meeting: on }));
          void write({ suggest_on_meeting: on });
        }}
      >
        {t("settings.suggest_on_meeting")}
      </Checkbox>
      <p className={HINT}>{t("settings.suggest_on_meeting_hint")}</p>

      {/* Which microphone.

          `recording.device_id` has been in the settings file since the daemon was written, and
          `Microphone` has accepted a `deviceId` for just as long. Nothing ever connected the two:
          the value was saved, reported back by `/settings`, and no recording ever read it — so
          somebody with a headset and a built-in microphone could name the one they wanted and be
          recorded by the other, with this screen showing their choice the whole time.

          Written to both stores, like the system-audio switch above and for the same reason: the
          recording reads `localStorage` because it has to open a device before any network call
          completes, and the daemon's copy is what this screen and the settings file show. */}
      <label className="mt-5 block">
        <span className={LABEL}>{t("settings.microphone")}</span>
        <Select
          className="mt-1 w-full sm:max-w-sm"
          value={now.device_id ?? ""}
          aria-label={t("settings.microphone")}
          data-testid="microphone"
          onChange={(event) => {
            const id = event.target.value;
            setLive((current) => ({ ...current, device_id: id }));
            saveCapture({ ...loadCapture(), device: id });
            void write({ device_id: id });
          }}
        >
          <option value="">{t("settings.microphone_default")}</option>
          {(devices.data ?? []).map((device, index) => (
            <option key={device.deviceId} value={device.deviceId}>
              {/* Anonymous until permission is granted. Numbered rather than blank, so a list of
                  three unnamed devices is still three things a person can choose between. */}
              {device.label.trim() || t("settings.microphone_unnamed", { n: index + 1 })}
            </option>
          ))}
        </Select>
        <span className={HINT}>{t("settings.microphone_hint")}</span>
      </label>

      {/* The shortcut that starts a recording without the window.

          `recording.hotkey` has been in the settings file since it had a schema: validated on the
          way in, saved, reported back by `/settings` — and read by nobody. The desktop shell
          registered a hardcoded combination, so changing this did exactly nothing, and on Windows
          and Linux the default it *showed* was not even the one that worked. Both halves are fixed;
          this is the half somebody can see.

          Typed rather than captured by listening for a keystroke. A capture control has to grab
          every key to work, which means it eats ⌘Q and Alt+F4 while it is focused — and it cannot
          express `CmdOrCtrl`, which is the whole reason the default is portable. */}
      <label className="mt-5 block">
        <span className={LABEL}>{t("settings.hotkey")}</span>
        <input
          type="text"
          value={now.hotkey ?? DEFAULT_HOTKEY}
          aria-label={t("settings.hotkey")}
          data-testid="hotkey"
          spellCheck={false}
          onChange={(event) => setLive((current) => ({ ...current, hotkey: event.target.value }))}
          onBlur={() => void writeHotkey(now.hotkey ?? DEFAULT_HOTKEY)}
          className="border-line bg-bg-soft text-fg rounded-card text-body mt-1 h-9 w-56 border px-2 font-mono"
        />
        <span className={HINT}>{t("settings.hotkey_hint")}</span>
      </label>

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
          className="border-line bg-bg-soft text-fg rounded-card text-body h-9 w-24 border px-2"
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
