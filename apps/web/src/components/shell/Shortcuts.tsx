import { m } from "motion/react";
import { useEffect } from "react";

import { useT } from "../../i18n/context";
import { GENTLE } from "../../lib/motion";
import { isMac } from "../../lib/shell";
import { Button } from "../ui";

/**
 * Every key the app answers to, on one card.
 *
 * The shortcuts existed and were written down nowhere a user would look: `⌘K` was a hint inside the
 * search box, `⌘⇧R` was in the README, and the number keys arrived with the menu bar. A shortcut
 * nobody can discover is a shortcut nobody uses.
 *
 * Opened from **Help → Keyboard shortcuts** in the menu bar, and from `?` — which is the key every
 * app with a shortcut sheet uses for it.
 */

/** The keys, as ids resolved at render. `mod` becomes ⌘ or Ctrl depending on the machine. */
const KEYS: { keys: string[]; labelKey: string }[] = [
  { keys: ["mod", "K"], labelKey: "shortcuts.search" },
  { keys: ["mod", "⇧", "R"], labelKey: "shortcuts.record" },
  { keys: ["mod", "N"], labelKey: "shortcuts.new_note" },
  { keys: ["mod", "O"], labelKey: "shortcuts.import" },
  { keys: ["mod", "B"], labelKey: "shortcuts.sidebar" },
  { keys: ["mod", ","], labelKey: "shortcuts.settings" },
  { keys: ["mod", "1"], labelKey: "shortcuts.home" },
  { keys: ["mod", "2"], labelKey: "shortcuts.library" },
  { keys: ["mod", "3"], labelKey: "shortcuts.tasks" },
  { keys: ["mod", "4"], labelKey: "shortcuts.analytics" },
  { keys: ["?"], labelKey: "shortcuts.this" },
];

export function Shortcuts({ onClose }: { onClose: () => void }) {
  const t = useT();
  const mod = isMac() ? "⌘" : "Ctrl";

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="fixed inset-0 z-50 grid place-items-center p-4">
      {/* A backdrop that closes it. A sheet with no way out but one small button is a sheet people
          get stuck in. */}
      <button
        type="button"
        aria-label={t("common.dismiss")}
        onClick={onClose}
        className="absolute inset-0 bg-black/30 backdrop-blur-[2px]"
      />
      <m.div
        role="dialog"
        aria-modal="true"
        aria-label={t("shortcuts.title")}
        initial={{ opacity: 0, y: 10, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={GENTLE}
        className="border-line bg-bg-elevated relative w-full max-w-lg rounded-[var(--radius-panel)] border p-5 shadow-[var(--shadow-pop)]"
      >
        <h2 className="text-title font-semibold">{t("shortcuts.title")}</h2>
        <p className="text-fg-dim text-meta mt-1">{t("shortcuts.lead")}</p>

        <ul className="mt-4 space-y-1.5">
          {KEYS.map((row) => (
            <li key={row.labelKey} className="flex items-center justify-between gap-4">
              <span className="text-body">{t(row.labelKey)}</span>
              <span className="flex shrink-0 items-center gap-1">
                {row.keys.map((key, at) => (
                  <kbd
                    key={at}
                    className="border-line bg-bg-soft text-micro tabular min-w-[26px] rounded-md border px-1.5 py-1 text-center"
                  >
                    {key === "mod" ? mod : key}
                  </kbd>
                ))}
              </span>
            </li>
          ))}
        </ul>

        <div className="mt-5 flex justify-end">
          <Button variant="primary" size="sm" onClick={onClose}>
            {t("common.dismiss")}
          </Button>
        </div>
      </m.div>
    </div>
  );
}
