import { createContext, useContext, type ReactNode } from "react";

import { flatten, type Catalog, type Language, type Translator } from ".";

/**
 * The active language, for the whole app.
 *
 * The choice is kept in `localStorage` rather than in the daemon's settings file. It is a property
 * of *this screen*, not of the vault: a Vietnamese team lead demoing on an English laptop should
 * not change what language their colleague's machine renders in, and the setting has to be readable
 * before the daemon handshake completes or the first paint is in the wrong language.
 */

export const STORAGE_KEY = "summo.locale";

export interface Value extends Translator {
  languages: Language[];
  setLocale: (locale: string) => void;
}

export const I18nContext = createContext<Value | null>(null);

/** Locale files the user added, keyed by tag. Fetched from the daemon; empty until it answers. */
export type ExtraLocales = Record<string, { label?: string; catalog: Catalog }>;

export interface Props {
  children: ReactNode;
  /** User-supplied catalogs, already fetched. Separate from the provider so tests can inject. */
  extra?: ExtraLocales;
  /** Force a locale, for tests and screenshots. */
  locale?: string;
}

export function read(): string | null {
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
