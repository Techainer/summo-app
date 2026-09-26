import { useCallback, useMemo, useState } from "react";

import { Checkbox } from "../ui";
import { CatalogueClient } from "../../lib/catalogue";
import { voicesFor } from "../../lib/dub";
import { useLoad } from "../../lib/use-load";
import { ListenIn } from "./ListenIn";
import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { TARGETS, hearsOthers, load, save, translating, type Capture } from "../../lib/capture";
import type { Lane } from "../../lib/protocol";
import { TranslateTargets } from "./TranslateTargets";
import { WarmUp } from "./WarmUp";
import { SpokenLanguage } from "./SpokenLanguage";

/**
 * What to listen to, and what language to put it in.
 *
 * These two switches are the whole "watch a talk in a language you do not speak" feature. There is
 * no companion window and no YouTube integration, because there is nothing to integrate with: the
 * system-audio loopback already hears whatever is playing. Turn on system audio, pick a language,
 * press play on anything.
 *
 * Both are disabled while recording. Changing lanes mid-session would mean tearing down and
 * rebuilding the pipeline underneath a running meeting, and changing the target language halfway
 * would leave a transcript subtitled in two languages with no way to tell which line is which.
 */
export function CaptureControls() {
  const { session, handshake } = useEngine();
  const { t } = useI18n();
  const [capture, setCapture] = useState<Capture>(() => load());

  /**
   * Which of the chosen target languages this machine can actually say.
   *
   * Two facts, both required: something is being translated into it, and a voice is installed that
   * speaks it. Offering a language on one of them produces the dead end this feature already had —
   * a control reporting it is on, over an hour of silence.
   *
   * Failing quietly is right. A catalogue that cannot be read costs the dub control, not the
   * recording, and the recording is the thing that cannot be done again.
   */
  const catalogue = useMemo(() => new CatalogueClient(handshake), [handshake]);
  const voices = useLoad(
    useCallback(async () => {
      try {
        return await catalogue.installed();
      } catch {
        return [];
      }
    }, [catalogue]),
    [catalogue],
  );
  const speakable = useMemo(
    () =>
      capture.translateInto
        .filter((code) => voicesFor(voices.data ?? [], code).length > 0)
        .map((code) => ({
          code,
          label: TARGETS.find((target) => target.code === code)?.label ?? code,
        })),
    [capture.translateInto, voices.data],
  );

  const update = (next: Capture) => {
    setCapture(next);
    save(next);
  };

  const toggleLane = (lane: Lane) => {
    const has = capture.lanes.includes(lane);
    const lanes = has ? capture.lanes.filter((l) => l !== lane) : [...capture.lanes, lane];
    // The daemon refuses a session with no lanes; dropping the last one would turn this into a
    // record button that fails.
    if (lanes.length === 0) return;
    update({ ...capture, lanes });

    // And tell the daemon, because the settings screen reads its copy.
    //
    // One fact with two homes: this control moved the lanes and the settings screen moved
    // `recording.capture_system_audio`, so each showed a state the other had not been told about
    // — and the one in Settings decided nothing at all. Best effort, and silent on failure: the
    // recording is already correct, and a toast about a settings write nobody asked for would be
    // noise in the middle of starting a meeting.
    if (lane === "system") {
      void fetch(`http://127.0.0.1:${handshake.port}/settings/recording?token=${handshake.token}`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ capture_system_audio: !has }),
      }).catch(() => {});
    }
  };

  const busy = session.recording;

  return (
    // No margin, no width of its own, no centring: it is a row inside whatever card or toolbar
    // puts it there. It used to centre itself in a `max-w-xl`, which is why the record screen had
    // its controls floating in the middle of a pane and its button somewhere else entirely.
    <div className="w-full">
      {/* No `ListeningIn` here. The shell draws it on every screen while a recording runs, so on
          this one it appeared twice — two banners saying the same sentence, each with its own
          "Đổi" button, and a click landing on whichever the browser found first. */}
      <WarmUp />

      <fieldset disabled={busy} className="mt-2 disabled:opacity-60">
        <legend className="sr-only">{t("record.audio_source")}</legend>

        <div className="flex flex-wrap items-center gap-2">
          {/* A real checkbox, hidden and styled through its own label.
           *
           * The browser's default control is a blue square drawn by the operating system; next to a
           * dark, green-accented interface it reads as something the page did not mean to include.
           * `appearance-none` on the input itself would leave the focus ring and the hit target to
           * rebuild by hand, whereas `sr-only` plus `peer-*` keeps every bit of native behaviour —
           * space to toggle, tab order, the accessibility tree — and changes only the paint. */}
          {(["mic", "system"] as Lane[]).map((lane) => (
            <Checkbox
              key={lane}
              className="border-line bg-bg-soft has-[:checked]:border-accent has-[:checked]:bg-accent-soft has-[:checked]:text-accent text-body rounded-full border px-3 py-1.5"
              checked={capture.lanes.includes(lane)}
              onChange={() => toggleLane(lane)}
            >
              {t(lane === "mic" ? "record.microphone" : "record.system")}
            </Checkbox>
          ))}

          {/* The language being spoken sits beside the lanes, because it is the same decision:
              what is going into the recording. The target language stays on the right, where it
              was, since it is a decision about the output. */}
          <SpokenLanguage
            value={capture.spoken}
            onChange={(spoken) => update({ ...capture, spoken })}
            compact
          />

          <span className="text-fg-faint text-body ms-auto flex items-center gap-2">
            {t("record.translate_live")}
            {/* The same control the running meeting uses, so a target added here and a target added
                mid-call are one idea rather than two dropdowns that behave differently. `TARGETS`
                already carries the empty "off" entry the shared control provides itself. */}
            <TranslateTargets
              value={capture.translateInto}
              options={TARGETS.filter((target) => target.code !== "")}
              onChange={(translateInto) => update({ ...capture, translateInto })}
            />
          </span>
        </div>

        {/* Hearing it, on its own row under reading it.
            Under rather than beside: it depends on the row above — there is nothing to speak until
            something is being translated — and a control that appears and disappears inside a line
            of other controls moves everything next to it every time somebody changes a language. */}
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-2">
          {speakable.length > 0 && (
            <span className="text-fg-faint text-body flex items-center gap-2">
              {t("record.listen_in")}
              <ListenIn
                value={capture.listenIn}
                volume={capture.listenVolume}
                options={speakable}
                onChange={(listenIn) => update({ ...capture, listenIn })}
                onVolume={(listenVolume) => update({ ...capture, listenVolume })}
              />
            </span>
          )}
        </div>
      </fieldset>

      {translating(capture) && (
        <p className="text-fg-dim text-meta mt-2">
          {/* The mistake this catches: translation on, system audio off, so the app dutifully
              translates the user's own voice back at them and looks broken. */}
          {hearsOthers(capture) ? t("record.translate_hint") : t("record.translate_needs_system")}
        </p>
      )}
    </div>
  );
}
