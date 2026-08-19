import * as DropdownMenu from "@radix-ui/react-dropdown-menu";
import { Check } from "lucide-react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";

/**
 * The interface language, by its own name.
 *
 * A dropdown rather than a cycle: there are four built-in languages and any number of user files in
 * `~/.summo/locales/`, and cycling through somebody else's Japanese to get back to Vietnamese is
 * not a control. The trigger is the tag — `VI`, `EN` — because a globe icon alone does not say
 * which language you are currently in, which is the one thing somebody in the wrong one needs.
 */
export function LanguageMenu() {
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
