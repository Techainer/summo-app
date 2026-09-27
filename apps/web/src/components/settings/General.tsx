import { useEffect, useState } from "react";

import { Checkbox, SegmentedControl, Select } from "../ui";
import { CONTROL, FIELD, HINT, LABEL } from "./fields";
import { useI18n, useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { SCHEMES, remember as rememberScheme } from "../../lib/theme";
import { useScheme } from "../../lib/use-scheme";
import { isOn as perfIsOn, onChange as onPerfChange, show as showPerf } from "../../lib/perf";
import { url } from "../../lib/library";
import { useMemory } from "../../lib/memory";

/**
 * The settings about the app rather than about the work: what language it speaks, whether it is
 * light or dark, and whether it shows what it is costing.
 */
export function General() {
  const t = useT();
  return (
    <div data-testid="settings-general">
      <p className="text-fg-faint text-meta mb-4 leading-normal">{t("settings.general_hint")}</p>
      <LanguagePicker />
      <AppearanceSetting />
      <PerformanceSetting />
    </div>
  );
}

/**
 * Whether to draw a readout of what Summo is costing.
 *
 * **Off by default.** A permanent gauge in the corner of a recorder is an invitation to watch a
 * number instead of a meeting. It is here for the person who wants to know what a background
 * daemon is doing on their laptop, and for them it should be one switch away — not something
 * everybody else has to look at.
 *
 * The switch writes to the vault *and* announces locally, which is what makes it and the × on the
 * panel itself the same switch. Without the announcement the panel would appear on the next
 * reload, which reads as a toggle that does not work.
 */
function PerformanceSetting() {
  const { handshake } = useEngine();
  const t = useT();
  const [on, setOn] = useState(perfIsOn);

  useEffect(() => onPerfChange(setOn), []);

  // What the vault says, adopted once. The local mirror is what avoids a round trip in front of
  // the first paint; this is what makes the choice survive a reload and reach a second window.
  useEffect(() => {
    let cancelled = false;
    fetch(url(handshake, "/settings"))
      .then((r) => r.json())
      .then((body: { settings?: { interface?: { show_performance?: boolean } } }) => {
        const said = body.settings?.interface?.show_performance;
        if (!cancelled && typeof said === "boolean" && said !== perfIsOn()) showPerf(said);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [handshake]);

  const set = (next: boolean) => {
    showPerf(next);
    // Fire-and-forget, like the theme: it is already applied, and a daemon that is not answering
    // must not make the switch fail.
    void fetch(url(handshake, "/settings/interface"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ show_performance: next }),
    }).catch(() => undefined);
  };

  return (
    <div className="mt-6">
      <Checkbox checked={on} onChange={set} data-testid="show-performance">
        {t("perf.setting")}
      </Checkbox>
      <p className="text-fg-faint text-micro mt-1.5 ml-6 leading-normal">
        {t("perf.setting_hint")}
      </p>

      {/* The numbers themselves, here, whether or not the footer is showing them.
          The footer readout was reported as noise in the chrome of every screen, so it is behind
          the switch above now — and hiding a number is only acceptable if it is still somewhere.
          This is that somewhere, next to the switch that decides it. */}
      <Usage />
    </div>
  );
}

/**
 * What this machine and this process are spending, as plain rows.
 *
 * Deliberately not a chart. The question somebody arrives with is "is it using too much" and three
 * numbers answer it; a graph would be an invitation to watch it.
 */
function Usage() {
  const { t } = useI18n();
  const { handshake, stat } = useEngine();
  const memory = useMemory(handshake);

  return (
    <dl className="border-line mt-4 ml-6 grid max-w-sm grid-cols-[1fr_auto] gap-x-6 gap-y-1.5 border-t pt-3">
      <dt className="text-fg-faint text-micro col-span-2 mb-0.5 tracking-wide uppercase">
        {t("perf.usage_title")}
      </dt>
      <dt className="text-fg-dim text-meta">{t("perf.usage_ram")}</dt>
      <dd className="tabular text-meta">
        {memory ? `${memory.usedGb.toFixed(1)} / ${memory.totalGb.toFixed(0)} GB` : "—"}
      </dd>

      <dt className="text-fg-dim text-meta">{t("perf.usage_process")}</dt>
      <dd className="tabular text-meta">{stat ? `${stat.rss_mb} MB` : "—"}</dd>

      <dt className="text-fg-dim text-meta">{t("perf.usage_rtf")}</dt>
      <dd className="tabular text-meta">{stat ? stat.rtf.toFixed(3) : "—"}</dd>

      {/* Said rather than left as three dashes: an empty row reads as broken, and "nothing is
          running" is the ordinary state of this screen. */}
      {!stat && <p className="text-fg-faint text-micro col-span-2 mt-1">{t("perf.usage_idle")}</p>}
    </dl>
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
      <p className={HINT}>{t("settings.language_hint")}</p>
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
  const { handshake } = useEngine();
  const t = useT();
  const scheme = useScheme();

  return (
    <div className={FIELD}>
      <span className={LABEL}>{t("theme.heading")}</span>
      <SegmentedControl
        label={t("theme.heading")}
        value={scheme}
        onChange={(next) => rememberScheme(handshake, next)}
        // Short labels here, the sentence in the palette. Three phrases like "Giao diện theo hệ
        // thống" side by side in a 390px column is a control that wraps out of its own pill — the
        // screenshot audit caught it as white text on nothing.
        options={SCHEMES.map((one) => ({ value: one, label: t(`theme.short_${one}`) }))}
      />
    </div>
  );
}
