import { useState } from "react";

import { Checkbox } from "../ui";
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
  const { session } = useEngine();
  const { t } = useI18n();
  const [capture, setCapture] = useState<Capture>(() => load());

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
              className="border-line bg-bg-soft has-[:checked]:border-accent has-[:checked]:bg-accent-soft has-[:checked]:text-accent rounded-full border px-3 py-1.5 text-sm"
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

          <span className="text-fg-faint ms-auto flex items-center gap-2 text-sm">
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
