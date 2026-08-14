import { useNavigate } from "@tanstack/react-router";
import { motion } from "motion/react";
import { CornerDownLeft, NotebookPen, Search, Waves } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { SNAPPY } from "../../lib/motion";
import { LibraryClient } from "../../lib/library";
import { asThings, matchPlaces, order, type Place, type Result } from "../../lib/palette";

/** How long to wait after the last keystroke before asking the daemon. */
const DEBOUNCE_MS = 140;

/**
 * Everywhere and everything, from one keystroke.
 *
 * Eleven destinations and a vault is more than a sidebar can make feel small. This is the single
 * affordance that fixes that: `⌘K` anywhere, type, press Enter. It searches *places* and *things*
 * in one list on purpose — a user should not have to know which of the two they want before they
 * are allowed to look for it.
 *
 * The vault half goes through `/library/search`, which has always covered recordings and typed
 * notes alike. The kind travels with each hit because it decides where opening it goes, and a note
 * sent to a meeting's route opens a page that does not exist.
 */
/**
 * Mounted only while open.
 *
 * The dialog used to stay mounted and return `null`, which meant its query, its results and its
 * cursor outlived the close — the browser test opened it, navigated, opened it again and typed the
 * second search onto the end of the first. That was patched with an effect that cleared three
 * pieces of state whenever `open` changed, which is a synchronous re-render for something React
 * does for free: unmount it, and there is no state left to be stale.
 */
export function Palette({ open, onClose }: { open: boolean; onClose: () => void }) {
  if (!open) return null;
  return <PaletteDialog onClose={onClose} />;
}

function PaletteDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const navigate = useNavigate();
  const { handshake } = useEngine();
  const library = useMemo(() => new LibraryClient(handshake), [handshake]);

  const [query, setQuery] = useState("");
  const [things, setThings] = useState<Result[]>([]);
  const [cursor, setCursor] = useState(0);
  const input = useRef<HTMLInputElement>(null);

  /**
   * The destinations, with the words somebody might reach for.
   *
   * English keywords beside Vietnamese labels because the interface language is not the language a
   * user's fingers default to: `models` has to find `Mô hình` in a Vietnamese interface.
   */
  const places: Place[] = useMemo(
    () => [
      { kind: "place", to: "/", label: t("nav.home"), keywords: ["home", "trang chinh"] },
      { kind: "place", to: "/record", label: t("nav.record"), keywords: ["record", "ghi", "thu"] },
      { kind: "place", to: "/library", label: t("nav.library"), keywords: ["vault", "kho"] },
      { kind: "place", to: "/tasks", label: t("nav.tasks"), keywords: ["tasks", "todo", "viec"] },
      { kind: "place", to: "/agenda", label: t("nav.agenda"), keywords: ["calendar", "lich"] },
      {
        kind: "place",
        to: "/analytics",
        label: t("nav.analytics"),
        keywords: ["analytics", "thong ke"],
      },
      { kind: "place", to: "/notes", label: t("nav.notes"), keywords: ["notes", "ghi chu"] },
      { kind: "place", to: "/chat", label: t("nav.chat"), keywords: ["ask", "hoi dap", "chat"] },
      { kind: "place", to: "/people", label: t("nav.people"), keywords: ["voices", "giong noi"] },
      { kind: "place", to: "/agents", label: t("nav.agents"), keywords: ["agents", "agent"] },
      { kind: "place", to: "/models", label: t("nav.models"), keywords: ["models", "mo hinh"] },
      {
        kind: "place",
        to: "/settings",
        label: t("nav.settings"),
        keywords: ["settings", "cai dat"],
      },
    ],
    [t],
  );

  // Fewer than two characters is not a short search, it is no search — so the results are read off
  // the query rather than stored as an empty list. `setThings([])` in the effect body was a
  // synchronous write on every keystroke back down to one character.
  const found = query.trim().length < 2 ? [] : things;

  useEffect(() => {
    if (query.trim().length < 2) return undefined;
    const timer = window.setTimeout(() => {
      library
        .search(query.trim(), 8)
        .then((hits) => setThings(asThings(hits)))
        // A failed search leaves the places, which are local and always right. Blanking the list
        // because the daemon hiccuped would take away the half that still works.
        .catch(() => setThings([]));
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [library, query]);

  const results = order(matchPlaces(places, query), found as never, query).slice(0, 12);

  const run = useCallback(
    (result: Result | undefined) => {
      if (!result) return;
      onClose();
      if (result.kind === "place") {
        void navigate({ to: result.to });
      } else if (result.entry === "note") {
        void navigate({ to: "/notes", search: { open: undefined } });
      } else {
        void navigate({ to: "/meetings/$meetingId", params: { meetingId: result.id } });
      }
    },
    [navigate, onClose],
  );

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/50 p-4 pt-[12vh]"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <motion.div
        initial={{ opacity: 0, y: -8, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={SNAPPY}
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.title")}
        data-testid="palette"
        className="border-line bg-bg-raised w-full max-w-xl overflow-hidden rounded-[var(--radius-panel)] border shadow-[var(--shadow-pop)]"
      >
        <div className="border-line flex items-center gap-2.5 border-b px-4 py-3">
          <Search aria-hidden="true" className="text-fg-faint size-4 shrink-0" />
          <input
            ref={input}
            // Mounted only while open, so the browser can focus it the ordinary way instead of an
            // effect reaching for a ref after the fact.
            autoFocus
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setCursor(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") onClose();
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setCursor((c) => Math.min(c + 1, results.length - 1));
              }
              if (event.key === "ArrowUp") {
                event.preventDefault();
                setCursor((c) => Math.max(c - 1, 0));
              }
              if (event.key === "Enter") {
                event.preventDefault();
                run(results[cursor]);
              }
            }}
            placeholder={t("palette.placeholder")}
            aria-label={t("palette.title")}
            className="text-body text-fg placeholder:text-fg-faint w-full bg-transparent focus:outline-none"
          />
          <kbd className="text-fg-faint text-micro border-line rounded border px-1.5 py-0.5">
            esc
          </kbd>
        </div>

        <ul className="max-h-[52vh] overflow-y-auto p-2">
          {results.length === 0 ? (
            <li className="text-fg-faint text-meta px-3 py-6 text-center">
              {t("palette.nothing")}
            </li>
          ) : (
            results.map((result, index) => (
              <li key={result.kind === "place" ? result.to : `${result.entry}-${result.id}`}>
                <button
                  type="button"
                  onMouseEnter={() => setCursor(index)}
                  onClick={() => run(result)}
                  aria-current={index === cursor ? "true" : undefined}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-[var(--radius-card)] px-3 py-2 text-left transition-colors",
                    index === cursor ? "bg-bg-elevated" : "hover:bg-bg-soft",
                  )}
                >
                  <span className="bg-bg-soft ring-line grid size-7 shrink-0 place-items-center rounded-full ring-1">
                    {result.kind === "place" ? (
                      <CornerDownLeft aria-hidden="true" className="text-fg-faint size-3.5" />
                    ) : result.entry === "note" ? (
                      <NotebookPen aria-hidden="true" className="text-fg-faint size-3.5" />
                    ) : (
                      <Waves aria-hidden="true" className="text-fg-faint size-3.5" />
                    )}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="text-body block truncate">
                      {result.kind === "place" ? result.label : result.title}
                    </span>
                    {result.kind === "thing" && result.excerpt && (
                      <span className="text-fg-faint text-micro block truncate">
                        {result.excerpt}
                      </span>
                    )}
                  </span>
                  {result.kind === "thing" && (
                    <span className="text-fg-faint tabular text-micro shrink-0">{result.day}</span>
                  )}
                </button>
              </li>
            ))
          )}
        </ul>
      </motion.div>
    </div>
  );
}
