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
import {
  BUILT_IN_LANGUAGES,
  catalogFor,
  detectLocale,
  ensure,
  ensureMore,
  mergeCatalogs,
  ready,
  translator,
  type Catalog,
  type Language,
} from "./index";

export function I18nProvider({ children, extra, locale: forced, preferred, onChosen }: Props) {
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

  /**
   * The vault's language, adopted on the one visit that has none of its own.
   *
   * `interface.language` has been in the settings file since it had a schema and nothing read it,
   * so choosing Japanese in one window told that window and nothing else — a second machine, or a
   * reinstall, went back to whatever the browser asked for.
   *
   * Only when `localStorage` is empty, and only through `setLocaleState` rather than `setLocale`:
   * writing it down here would turn "the vault prefers Japanese" into "this browser has chosen
   * Japanese", and the next change made elsewhere would stop arriving. It also arrives after first
   * paint by necessity — it comes over the network — so this is a switch, not the initial value.
   */
  useEffect(() => {
    if (!preferred || read() !== null) return undefined;
    if (!languages.some((language) => language.code === preferred)) return undefined;
    let live = true;
    void ensure(preferred).then(() => {
      if (live) setLocaleState(preferred);
    });
    return () => {
      live = false;
    };
  }, [preferred, languages]);

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

  /**
   * The shipped strings for this locale, once the file holding them has arrived.
   *
   * State rather than a call in the render, because the catalogs are a module-level cache that
   * React has nothing to subscribe to. `null` is the one state a screen must not be painted in —
   * see the gate at the bottom — and it lasts as long as one fetch of a few kB.
   *
   * Set from the cache when it is already warm, which is the ordinary case: `main.tsx` starts the
   * fetch before React mounts and the engine handshake in front of this takes far longer.
   */
  const [shipped, setShipped] = useState<Catalog | null>(() =>
    ready(active) ? catalogFor(active) : null,
  );

  useEffect(() => {
    let live = true;
    // Resolves immediately when the cache is warm, and resolves *anyway* when the fetch failed —
    // so this always reaches a catalog, even if that catalog is empty. A language file that will
    // not load leaves key names on screen, which is visibly wrong; waiting forever leaves nothing.
    void ensure(active).then(() => {
      if (live) setShipped(catalogFor(active));
      // And then the half only lazy screens need, merged in when it lands. Not awaited with the
      // first: the point of the split is that the app paints without it.
      void ensureMore(active).then(() => {
        if (live) setShipped(catalogFor(active));
      });
    });
    return () => {
      live = false;
    };
  }, [active]);

  const value = useMemo<Value>(() => {
    // The user's own file on top, which is where a half-translated contributed locale gets the
    // rest of its strings from.
    const catalog = mergeCatalogs(shipped ?? {}, extra?.[active]?.catalog ?? {});
    const base = translator(active, catalog, (key) => {
      // Once per key per session; the translator itself dedupes.
      console.warn(`[i18n] missing key: ${key}`);
    });
    return {
      ...base,
      languages,
      setLocale: (next: string) => {
        try {
          window.localStorage.setItem(STORAGE_KEY, next);
        } catch {
          // Private browsing and locked-down webviews both throw here. The choice still applies to
          // this session; it just will not be remembered.
        }
        // And the vault, so the choice survives this browser. Fire-and-forget: the language has
        // already changed here, and a daemon that is not answering must not make the switch fail.
        onChosen?.(next);
        // Fetched *before* the switch, not after. Setting the locale first would render one frame
        // against a catalog that is not there yet — a flash of key names, or of Vietnamese, on the
        // way to the language somebody just chose. The chunk is a few kB from the same origin, so
        // what this costs is imperceptible and what it buys is that the screen changes once.
        void ensure(next).then(() => setLocaleState(next));
      },
    };
  }, [active, extra, languages, shipped, onChosen]);

  useEffect(() => {
    // Screen readers announce in the wrong language without this, and CSS `:lang()` cannot match.
    document.documentElement.lang = active;
  }, [active]);

  /**
   * The language the download page was written in, offered once by a `summo://lang/<code>` link.
   *
   * This is the answer to a narrow question: somebody who reads Vietnamese, on a Mac set to
   * English, downloading from the Vietnamese page. The operating system says English and is the
   * only signal the app otherwise has, so it opens in English — and the one piece of evidence to
   * the contrary is the page they were standing on, which is what the link carries over.
   *
   * **It never overrides a choice.** A saved value in storage means the user picked a language, in
   * settings or in setup, and a link from a web page does not get to undo that — otherwise
   * revisiting the download page and clicking the wrong button silently changes the app somebody
   * has been using for a month. So it applies only when nothing has been saved, which is exactly
   * the first-run case it exists for. `setLocale` then writes it, so the offer is not repeatable.
   *
   * A language the app does not ship is ignored rather than fallen back on: falling back would set
   * English on a first run whose OS locale had already chosen better.
   */
  useEffect(() => {
    const onOffer = (event: Event) => {
      const code = (event as CustomEvent<string>).detail;
      if (read()) return;
      if (!languages.some((language) => language.code === code)) return;
      value.setLocale(code);
    };
    window.addEventListener("summo:set-locale", onOffer);
    return () => window.removeEventListener("summo:set-locale", onOffer);
  }, [languages, value]);

  // Nothing renders against a catalog that has not arrived. An app painted from an empty one would
  // show its own key names — `nav.record`, `meeting.stop` — and replace them a frame later, which
  // reads as a glitch rather than as loading.
  //
  // After every hook, never before one.
  if (shipped === null) return null;

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}
