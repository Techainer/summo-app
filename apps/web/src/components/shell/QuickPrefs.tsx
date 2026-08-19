import { Monitor, Moon, Sun } from "lucide-react";
import { Suspense, lazy, useState } from "react";

import { useI18n } from "../../i18n/context";
import { choose, read, type Scheme } from "../../lib/theme";

/**
 * Fetched when the header renders, not before it.
 *
 * The menu is a Radix dropdown, which is 19 kB gzipped, and putting it in the entry chunk took the
 * first load from 195 kB to 214 kB — past a budget that exists precisely so a control nobody has
 * clicked yet cannot slow down the first paint for everybody. `MenuBar` is lazy for the same reason
 * and for the same 19 kB. The theme button below has no dependency at all and stays where it is.
 */
const LanguageMenu = lazy(() =>
  import("./LanguageMenu").then((m) => ({ default: m.LanguageMenu })),
);

/**
 * Language and light-or-dark, in the top bar.
 *
 * Both settings existed and both were two navigations away — Cài đặt → Chung — or in ⌘K, which is a
 * place you find things you already know are there. They are the two preferences somebody changes
 * *while looking at the screen they are wrong on*: the app opened in a language they do not read,
 * or the room got dark. A setting like that belongs where the eyes already are.
 *
 * Kept small on purpose. The header carries a record button and a live meter; two icon-sized
 * controls fit beside them, a segmented control with three labels does not.
 */
export function QuickPrefs() {
  return (
    <>
      <ThemeButton />
      <Suspense fallback={null}>
        <LanguageMenu />
      </Suspense>
    </>
  );
}

const ICONS: Record<Scheme, typeof Monitor> = {
  system: Monitor,
  light: Sun,
  dark: Moon,
};

/** The cycle, written down rather than computed from an index into {@link SCHEMES}. */
const NEXT: Record<Scheme, Scheme> = {
  system: "light",
  light: "dark",
  dark: "system",
};

/**
 * One button, three states, cycling.
 *
 * A dropdown for three options is a menu to open, aim at and dismiss for something that is nearly
 * always "the other one". The icon says which state it is in and the tooltip names it, so the cycle
 * is not a guess — and `system` is in the loop rather than hidden in Settings, because the default
 * has to be reachable by the person who left it.
 */
function ThemeButton() {
  const { t } = useI18n();
  const [scheme, setScheme] = useState<Scheme>(() => read());
  const Icon = ICONS[scheme];
  const next = NEXT[scheme];

  return (
    <button
      type="button"
      onClick={() => {
        choose(next);
        setScheme(next);
      }}
      aria-label={t("theme.heading")}
      title={t(`theme.${scheme}`)}
      className="text-fg-faint hover:bg-bg-soft hover:text-fg hidden rounded-lg px-2 py-1.5 transition-colors sm:block"
    >
      <Icon aria-hidden="true" className="size-4 stroke-[1.75]" />
    </button>
  );
}
