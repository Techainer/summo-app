import { useState } from "react";

import { SegmentedControl, Select } from "../ui";
import { CONTROL, FIELD, LABEL } from "./fields";
import { useI18n, useT } from "../../i18n/context";
import { SCHEMES, choose as chooseScheme, read as readScheme, type Scheme } from "../../lib/theme";

/**
 * The two settings about the app rather than about the work: what language it speaks, and whether
 * it is light or dark.
 */
export function General() {
  const t = useT();
  return (
    <div data-testid="settings-general">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.general_hint")}</p>
      <LanguagePicker />
      <AppearanceSetting />
    </div>
  );
}

/**
 * Which language the interface is in.
 *
 * Listed by each language's own name — somebody looking for their language cannot read a list
 * written in a language they do not read, which is why "Tiếng Việt" is not "Vietnamese".
 *
 * The hint about `~/.summo/locales/` is the whole contribution process, so it belongs on screen
 * rather than in a document nobody opens.
 */
function LanguagePicker() {
  const { locale, setLocale, languages, t } = useI18n();

  return (
    <>
      <label className={FIELD}>
        <span className={LABEL}>{t("settings.language")}</span>
        <Select
          className={CONTROL}
          value={locale}
          aria-label={t("settings.language")}
          onChange={(e) => setLocale(e.target.value)}
        >
          {languages.map((language) => (
            <option key={language.code} value={language.code}>
              {language.label}
            </option>
          ))}
        </Select>
      </label>
      <p className="text-fg-faint text-micro mt-1.5 ml-[162px] leading-normal">
        {t("settings.language_hint")}
      </p>
    </>
  );
}

/**
 * Light, dark, or whatever the machine says.
 *
 * Here as well as in ⌘K, because a preference that only exists in a command palette is one most
 * people never find — and because this is the screen somebody opens when they are looking for a
 * setting rather than for a shortcut.
 */
function AppearanceSetting() {
  const t = useT();
  const [scheme, setScheme] = useState<Scheme>(() => readScheme());

  return (
    <div className={FIELD}>
      <span className={LABEL}>{t("theme.heading")}</span>
      <SegmentedControl
        label={t("theme.heading")}
        value={scheme}
        onChange={(next) => {
          chooseScheme(next);
          setScheme(next);
        }}
        // Short labels here, the sentence in the palette. Three phrases like "Giao diện theo hệ
        // thống" side by side in a 390px column is a control that wraps out of its own pill — the
        // screenshot audit caught it as white text on nothing.
        options={SCHEMES.map((one) => ({ value: one, label: t(`theme.short_${one}`) }))}
      />
    </div>
  );
}
