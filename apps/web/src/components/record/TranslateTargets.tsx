import { X } from "lucide-react";

import { useT } from "../../i18n/context";
import { Select } from "../ui";

/**
 * The languages a recording is being translated into, and a way to add or drop one.
 *
 * A meeting can have more than one reader. A Vietnamese standup with a Japanese contractor and an
 * English investor on the call needed the transcript twice, and a single dropdown made that a
 * choice about whose subtitle mattered — for no good reason: SMALL100 is one multilingual model and
 * the target is a token it starts with, so the second language is another pass through weights that
 * are already in memory rather than another six hundred megabytes.
 *
 * A select that adds, plus a chip per target that removes, rather than a `<select multiple>`. The
 * native multiple-select renders as a scrolling box that needs ctrl-click to deselect, which is
 * unusable in a banner over a running meeting and unknown to most of the people using one — and it
 * gives no way to see the current answer at a glance, which is the thing this control exists to
 * show.
 *
 * The empty option means off, and it clears all of them at once. Off is a state somebody reaches in
 * a hurry, usually because subtitles are in the way; making them remove three chips to get there
 * would be the same mistake as having no off at all.
 */
export function TranslateTargets({
  value,
  options,
  onChange,
  disabled,
  size = "sm",
}: {
  value: string[];
  /** Everything selectable, already ordered and labelled for the reader's locale. */
  options: { code: string; label: string }[];
  onChange: (next: string[]) => void;
  disabled?: boolean;
  size?: "sm" | "md";
}) {
  const t = useT();
  const label = t("record.translate_live");

  // Only what is not already on. An option that is already a chip below it would either do nothing
  // or silently duplicate a subtitle, and the daemon deduplicates precisely because a control could
  // let that happen.
  const addable = options.filter((option) => !value.includes(option.code));

  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      <Select
        size={size}
        aria-label={label}
        // Always the empty option: this select *adds*, it does not hold the answer. The answer is
        // the chips, because there can be several and a select can show one.
        value=""
        disabled={disabled}
        onChange={(event) => {
          const code = event.target.value;
          onChange(code === "" ? [] : [...value, code]);
        }}
      >
        <option value="">
          {value.length === 0 ? t("record.translate_off") : t("record.translate_all_off")}
        </option>
        {addable.map((option) => (
          <option key={option.code} value={option.code}>
            {option.label}
          </option>
        ))}
      </Select>

      {value.map((code) => (
        <button
          key={code}
          type="button"
          // Removing one, not opening a menu about it. The chip is the control.
          onClick={() => onChange(value.filter((each) => each !== code))}
          className="border-line text-fg-dim hover:text-fg hover:border-fg-faint text-micro inline-flex items-center gap-1 rounded-full border px-2 py-0.5 transition-colors"
          aria-label={t("record.translate_drop", {
            language: options.find((option) => option.code === code)?.label ?? code,
          })}
        >
          {options.find((option) => option.code === code)?.label ?? code}
          <X aria-hidden="true" className="size-3" />
        </button>
      ))}
    </span>
  );
}
