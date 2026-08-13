import { useEffect, useState, type ReactNode } from "react";

import { useEngine } from "../lib/engine-context";
import { url } from "../lib/library";
import { parseLocales, type ExtraLocales } from "./context";
import { I18nProvider } from "./provider";

/**
 * The translation provider, with the user's own locale files loaded in.
 *
 * Rendering happens immediately with the built-in languages rather than waiting for the daemon.
 * Blocking first paint on a network call to fetch translations for a language most users do not
 * have would trade a real cost for a hypothetical one; the extra languages appear a moment later,
 * and the saved-choice effect in {@link I18nProvider} switches to one if that is what was selected.
 *
 * This sits *inside* `EngineProvider` because it needs the handshake, and *outside* the router so
 * that navigating does not refetch.
 */
export function AppI18n({ children }: { children: ReactNode }) {
  const { handshake } = useEngine();
  const [extra, setExtra] = useState<ExtraLocales>({});

  useEffect(() => {
    let cancelled = false;
    fetch(url(handshake, "/locales"))
      .then((response) => (response.ok ? response.json() : {}))
      .then((payload) => {
        if (!cancelled) setExtra(parseLocales(payload));
      })
      .catch(() => {
        // A daemon that is not up yet costs the user their added languages for this session, not
        // the app. The built-in ones still work.
      });
    return () => {
      cancelled = true;
    };
  }, [handshake]);

  return <I18nProvider extra={extra}>{children}</I18nProvider>;
}
