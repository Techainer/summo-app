import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

import {
  BUILT_IN_LANGUAGES,
  catalogFor,
  detectLocale,
  flatten,
  translator,
  type Catalog,
  type Language,
  type Translator,
} from ".";

/**
 * The active language, for the whole app.
 *
 * The choice is kept in `localStorage` rather than in the daemon's settings file. It is a property
 * of *this screen*, not of the vault: a Vietnamese team lead demoing on an English laptop should
 * not change what language their colleague's machine renders in, and the setting has to be readable
 * before the daemon handshake completes or the first paint is in the wrong language.
 */

const STORAGE_KEY = "summo.locale";

interface Value extends Translator {
  languages: Language[];
  setLocale: (locale: string) => void;
}

const I18nContext = createContext<Value | null>(null);

/** Locale files the user added, keyed by tag. Fetched from the daemon; empty until it answers. */
export type ExtraLocales = Record<string, { label?: string; catalog: Catalog }>;

interface Props {
  children: ReactNode;
  /** User-supplied catalogs, already fetched. Separate from the provider so tests can inject. */
  extra?: ExtraLocales;
  /** Force a locale, for tests and screenshots. */
  locale?: string;
}

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

  const active = forced ?? locale;

  // A user-added language that arrives after first paint must be selectable, and a saved choice
  // pointing at one must take effect once it loads rather than being silently ignored.
  useEffect(() => {
    const saved = read();
    if (saved && saved !== locale && languages.some((l) => l.code === saved)) {
      setLocaleState(saved);
    }
  }, [languages, locale]);

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

function read(): string | null {
  try {
    return window.localStorage.getItem(STORAGE_KEY);
  } catch {
    return null;
  }
}

/**
 * The translator.
 *
 * Throws outside a provider rather than falling back to a default one. A component rendering
 * untranslated because somebody forgot the provider is the failure that ships; a component that
 * refuses to render is the failure that gets fixed.
 */
export function useI18n(): Value {
  const value = useContext(I18nContext);
  if (!value) throw new Error("useI18n called outside <I18nProvider>");
  return value;
}

/** Shorthand for the common case. */
export function useT(): Translator["t"] {
  return useI18n().t;
}

/** Turn the daemon's `/locales` response into catalogs. */
export function parseLocales(payload: unknown): ExtraLocales {
  if (typeof payload !== "object" || payload === null) return {};
  const out: ExtraLocales = {};
  for (const [code, value] of Object.entries(payload as Record<string, unknown>)) {
    if (typeof value !== "object" || value === null) continue;
    const record = value as { label?: unknown; strings?: unknown };
    out[code] = {
      label: typeof record.label === "string" ? record.label : undefined,
      catalog: flatten(record.strings ?? value),
    };
  }
  return out;
}
