import { Headphones, Volume2, VolumeX } from "lucide-react";

import { useT } from "../../i18n/context";
import { Select } from "../ui";

/**
 * Hearing the meeting in another language, rather than reading it.
 *
 * A different verb from the control beside it, because it is a different thing. *Translate into* is
 * a list — a meeting can have several readers and two subtitles share a screen happily. *Listen in*
 * is one language, because a person has one pair of ears and two voices over each other is nobody's
 * dub.
 *
 * ## Only what can actually be spoken
 *
 * The options are the intersection of two facts: a language something is being translated into, and
 * a language a voice on this machine can say. Offering anything else produces the dead end this
 * feature already had once — a control that reports it is on, over an hour of silence.
 *
 * That is why `options` is passed in rather than derived here. Which voices are installed is a
 * question for the daemon, and a control that answered it from a guess would be offering a language
 * on the strength of hoping.
 *
 * ## The headphones line
 *
 * Not decoration. This app captures system audio, so a dub coming out of the speakers is captured,
 * transcribed, translated and spoken again. The daemon watches for that and says so if it happens;
 * this is the sentence that stops it happening.
 */
export function ListenIn({
  value,
  volume,
  options,
  onChange,
  onVolume,
  disabled,
  size = "sm",
}: {
  /** The language being spoken, or empty for off. */
  value: string;
  /** `0..=1`. */
  volume: number;
  /** Languages that are both being translated into and have an installed voice. */
  options: { code: string; label: string }[];
  onChange: (next: string) => void;
  onVolume: (next: number) => void;
  disabled?: boolean;
  size?: "sm" | "md";
}) {
  const t = useT();
  const on = value !== "" && options.some((option) => option.code === value);

  // Nothing can be spoken, so there is nothing to offer. Silent rather than disabled: a control
  // that is permanently greyed out in a bar somebody reads every meeting is noise, and the place
  // that explains how to get a voice is the dub panel, which says which one and how big.
  if (options.length === 0) return null;

  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      <Select
        size={size}
        aria-label={t("record.listen_in")}
        value={on ? value : ""}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      >
        <option value="">{t("record.listen_off")}</option>
        {options.map((option) => (
          <option key={option.code} value={option.code}>
            {option.label}
          </option>
        ))}
      </Select>

      {/* The volume and the warning appear together with the thing they are about. A slider for a
          dub nobody asked for is a control with no subject. */}
      {on && (
        <>
          <label className="inline-flex items-center gap-1.5">
            <span className="sr-only">{t("record.listen_volume")}</span>
            {volume === 0 ? (
              <VolumeX aria-hidden="true" className="text-fg-faint size-3.5" />
            ) : (
              <Volume2 aria-hidden="true" className="text-fg-faint size-3.5" />
            )}
            <input
              type="range"
              min={0}
              max={100}
              step={5}
              value={Math.round(volume * 100)}
              disabled={disabled}
              onChange={(event) => onVolume(Number(event.target.value) / 100)}
              className="accent-accent h-1 w-20 cursor-pointer"
              data-testid="listen-volume"
            />
          </label>

          <span className="text-fg-faint text-micro inline-flex items-center gap-1">
            <Headphones aria-hidden="true" className="size-3" />
            {t("record.listen_headphones")}
          </span>
        </>
      )}
    </span>
  );
}
