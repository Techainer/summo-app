import { useEffect } from "react";

/**
 * `⌘K` / `Ctrl-K`, from anywhere.
 *
 * A hook rather than a listener inside the dialog, because the dialog is not mounted when the
 * shortcut has to work — which is the whole point of it.
 */
export function usePaletteShortcut(onOpen: () => void) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        onOpen();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onOpen]);
}
