import { useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { TARGETS, hearsOthers, load, save, translating, type Capture } from "../../lib/capture";
import type { Lane } from "../../lib/protocol";

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
    <div className="mx-auto mt-6 w-full max-w-xl px-4">
      <fieldset disabled={busy} className="disabled:opacity-60">
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
            <label
              key={lane}
              className="group flex cursor-pointer items-center gap-2 rounded-full border border-line bg-bg-soft px-3 py-1.5 text-sm text-fg-dim transition-colors has-[:checked]:border-accent has-[:checked]:bg-accent-soft has-[:checked]:text-accent has-[:focus-visible]:border-accent"
            >
              <input
                type="checkbox"
                className="peer sr-only"
                checked={capture.lanes.includes(lane)}
                onChange={() => toggleLane(lane)}
              />
              <span
                aria-hidden="true"
                className="grid size-4 place-items-center rounded-[5px] border border-line-strong text-[10px] text-accent-fg peer-checked:border-accent peer-checked:bg-accent"
              >
                <span className="opacity-0 transition-opacity group-has-[:checked]:opacity-100">✓</span>
              </span>
              {t(lane === "mic" ? "record.microphone" : "record.system")}
            </label>
          ))}

          <label className="ms-auto flex items-center gap-2 text-sm text-fg-faint">
            {t("record.translate_live")}
            <select
              value={capture.translateTo}
              aria-label={t("record.translate_live")}
              onChange={(e) => update({ ...capture, translateTo: e.target.value })}
              className="rounded-lg border border-line bg-bg-soft px-2 py-1.5 text-sm text-fg focus:outline-none focus-visible:border-accent"
            >
              {TARGETS.map((target) => (
                <option key={target.code} value={target.code}>
                  {target.label}
                </option>
              ))}
            </select>
          </label>
        </div>
      </fieldset>

      {translating(capture) && (
        <p className="mt-2 text-[13px] text-fg-dim">
          {/* The mistake this catches: translation on, system audio off, so the app dutifully
              translates the user's own voice back at them and looks broken. */}
          {hearsOthers(capture) ? t("record.translate_hint") : t("record.translate_needs_system")}
        </p>
      )}
    </div>
  );
}
