import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Check, Monitor, Moon, Sun } from "lucide-react";
import { useState } from "react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { choose, read, type Scheme } from "../../lib/theme";

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
      <LanguageMenu />
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

/**
 * The interface language, by its own name.
 *
 * A dropdown rather than a cycle: there are four built-in languages and any number of user files in
 * `~/.summo/locales/`, and cycling through somebody else's Japanese to get back to Vietnamese is
 * not a control. The trigger is the tag — `VI`, `EN` — because a globe icon alone does not say
 * which language you are currently in, which is the one thing somebody in the wrong one needs.
 */
function LanguageMenu() {
  const { locale, setLocale, languages, t } = useI18n();

  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger
        aria-label={t("settings.language")}
        title={t("settings.language")}
        className="text-fg-faint hover:bg-bg-soft hover:text-fg data-[state=open]:bg-bg-soft text-micro hidden rounded-lg px-2 py-1.5 font-medium tracking-wide uppercase transition-colors sm:block"
      >
        {locale.split("-")[0]}
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content
          sideOffset={4}
          align="end"
          className="border-line bg-bg-elevated z-50 min-w-44 rounded-[var(--radius-card)] border p-1 shadow-[var(--shadow-pop)]"
        >
          {languages.map((language) => (
            <DropdownMenu.Item
              key={language.code}
              onSelect={() => setLocale(language.code)}
              className={cn(
                "text-body data-[highlighted]:bg-bg-soft flex cursor-pointer items-center justify-between gap-6 rounded-md px-2 py-1.5 outline-none",
                language.code === locale && "font-medium",
              )}
            >
              {language.label}
              {language.code === locale && (
                <Check aria-hidden="true" className="text-accent size-3.5" />
              )}
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
