/**
 * The provider, apart from the hooks.
 *
 * Fast refresh replaces a module's exports when it changes; it can only keep component state across
 * that swap if the module exports components and nothing else. A file holding both the provider and
 * `useT` forces a full reload on every edit — the whole app remounts and whatever was on screen is
 * lost, which is exactly the cost fast refresh exists to avoid. So the component lives here and the
 * hooks stay in `context.tsx`.
 */
import { useEffect, useMemo, useState } from "react";

import { I18nContext, STORAGE_KEY, read, type Props, type Value } from "./context";
import { BUILT_IN_LANGUAGES, catalogFor, detectLocale, translator, type Language } from "./index";

export function I18nProvider({ children, extra, locale: forced }: Props) {
  const languages = useMemo<Language[]>(() => {
    const added = Object.entries(extra ?? {})
      .filter(([code]) => !BUILT_IN_LANGUAGES.some((l) => l.code === code))
      .map(([code, value]) => ({ code, label: value.label ?? code }));
    return [...BUILT_IN_LANGUAGES, ...added];
  }, [extra]);

  const [locale, setLocaleState] = useState(() =>
    detectLocale(
      languages.map((l) => l.code),
      read(),
    ),
  );

  // A user-added language that arrives after first paint must be selectable, and a saved choice
  // pointing at one must take effect once it loads rather than being silently ignored.
  //
  // Computed here rather than set from an effect. The saved value is already on disk before the
  // first render — there is nothing to wait for — so an effect only bought a second render in the
  // wrong language, visible as a flash of English on a machine set to Vietnamese.
  const saved = read();
  const restored =
    saved && saved !== locale && languages.some((language) => language.code === saved)
      ? saved
      : locale;
  const active = forced ?? restored;

  const value = useMemo<Value>(() => {
    const catalog = catalogFor(active, extra?.[active]?.catalog);
    const base = translator(active, catalog, (key) => {
      // Once per key per session; the translator itself dedupes.
      console.warn(`[i18n] missing key: ${key}`);
    });
    return {
      ...base,
      languages,
      setLocale: (next: string) => {
        setLocaleState(next);
        try {
          window.localStorage.setItem(STORAGE_KEY, next);
        } catch {
          // Private browsing and locked-down webviews both throw here. The choice still applies to
          // this session; it just will not be remembered.
        }
      },
    };
  }, [active, extra, languages]);

  useEffect(() => {
    // Screen readers announce in the wrong language without this, and CSS `:lang()` cannot match.
    document.documentElement.lang = active;
  }, [active]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}
