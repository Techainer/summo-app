import { Plus, X } from "lucide-react";

import { useT } from "../../i18n/context";
import { Select } from "../ui";

/**
 * The languages a recording is being translated into.
 *
 * ## The mistake this is a correction of
 *
 * The first version of this control was a dropdown that always read "Off" and a row of chips
 * underneath holding the real answer. It was tidy and it was wrong: you chose Vietnamese, the box
 * still said `Tắt`, and the only sign anything had happened was a chip you had not been looking at.
 * Measured on a real meeting with two targets running and subtitles visibly arriving, the control
 * read **"Tắt — mọi ngôn ngữ"**. A control that reports "off" over a feature that is working is
 * worse than one that cannot express the feature at all.
 *
 * So the dropdown holds the answer again, exactly as it did before multiple targets existed:
 * choosing a language *is* the language, and choosing the empty option turns translation off. That
 * is the whole interaction for the people who want one subtitle, which is nearly everybody.
 *
 * A second language is a second step, taken from the `+` beside it, and only then do chips appear —
 * for the extras, never for the one the dropdown is already showing. Nothing about the common case
 * changed; the uncommon one is additive.
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

  const [primary, ...extras] = value;
  const nameOf = (code: string) => options.find((o) => o.code === code)?.label ?? code;
  // Only what is not already on. Offering a language twice would either do nothing or ask the
  // daemon for a subtitle it deduplicates anyway.
  const addable = options.filter((option) => !value.includes(option.code));

  return (
    <span className="inline-flex flex-wrap items-center gap-1.5">
      <Select
        size={size}
        aria-label={label}
        value={primary ?? ""}
        disabled={disabled}
        onChange={(event) => {
          const code = event.target.value;
          // Empty clears everything, including the extras. Off is a state somebody reaches in a
          // hurry — usually because subtitles are in the way — and making them dismiss three chips
          // to get there would be the same mistake in a smaller place.
          onChange(code === "" ? [] : [code, ...extras.filter((each) => each !== code)]);
        }}
      >
        <option value="">{t("record.translate_off")}</option>
        {options.map((option) => (
          <option key={option.code} value={option.code}>
            {option.label}
          </option>
        ))}
      </Select>

      {/* A second reader, when there is one. Hidden until a first language is chosen: "add another"
          is not a sentence that means anything before there is one to add to. */}
      {primary !== undefined && addable.length > 0 && (
        <label className="inline-flex items-center">
          <span className="sr-only">{t("record.translate_add")}</span>
          <span className="relative inline-flex items-center">
            <Plus
              aria-hidden="true"
              className="text-fg-faint pointer-events-none absolute left-1.5 size-3"
            />
            <Select
              size={size}
              aria-label={t("record.translate_add")}
              value=""
              disabled={disabled}
              className="w-14 ps-5"
              onChange={(event) => {
                if (event.target.value) onChange([...value, event.target.value]);
              }}
            >
              <option value="" />
              {addable.map((option) => (
                <option key={option.code} value={option.code}>
                  {option.label}
                </option>
              ))}
            </Select>
          </span>
        </label>
      )}

      {extras.map((code) => (
        <button
          key={code}
          type="button"
          onClick={() => onChange(value.filter((each) => each !== code))}
          className="border-line text-fg-dim hover:text-fg hover:border-fg-faint text-micro inline-flex items-center gap-1 rounded-full border px-2 py-0.5 transition-colors"
          aria-label={t("record.translate_drop", { language: nameOf(code) })}
        >
          {nameOf(code)}
          <X aria-hidden="true" className="size-3" />
        </button>
      ))}
    </span>
  );
}
