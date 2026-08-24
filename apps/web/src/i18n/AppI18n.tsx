import { useCallback, useEffect, useState, type ReactNode } from "react";

import { useEngine } from "../lib/engine-context";
import { url } from "../lib/library";
import { adopt } from "../lib/theme";
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
  const [preferred, setPreferred] = useState<string | undefined>(undefined);

  /**
   * The preferences the vault has been keeping and nobody read.
   *
   * `interface.theme` and `interface.language` have been in `settings.json`, validated and
   * defaulted, since it had a schema — and neither the daemon, the app nor the shell ever looked at
   * them. The browser held its own copy, so a choice reached the window that made it and stopped
   * there: a second window, another machine and a reinstall all started over.
   *
   * Read here because this component already has the handshake and already sits between the engine
   * and the router. Read *after* first paint, deliberately: putting a round trip in front of the
   * first pixel to find out what colour it should be is a worse trade than one late switch on the
   * single visit that has nothing saved locally.
   */
  useEffect(() => {
    let cancelled = false;
    fetch(url(handshake, "/settings"))
      .then((response) => (response.ok ? response.json() : null))
      .then(
        (payload: { settings?: { interface?: { theme?: string; language?: string } } } | null) => {
          if (cancelled) return;
          const chosen = payload?.settings?.interface;
          adopt(chosen?.theme);
          // Empty is "nobody has chosen", which must not read as a language.
          setPreferred(chosen?.language?.trim() || undefined);
        },
      )
      .catch(() => {
        // The app works from what this browser knows. That is where it stood before this existed.
      });
    return () => {
      cancelled = true;
    };
  }, [handshake]);

  const remember = useCallback(
    (locale: string) => {
      void fetch(url(handshake, "/settings/interface"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ language: locale }),
      }).catch(() => undefined);
    },
    [handshake],
  );

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

  return (
    <I18nProvider extra={extra} preferred={preferred} onChosen={remember}>
      {children}
    </I18nProvider>
  );
}
