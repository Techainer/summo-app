import { PlugZap } from "lucide-react";

import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { useReachable } from "../../lib/reachable";

/**
 * The bar that appears when the daemon stops answering.
 *
 * Everything in this app is a request to a process on the same machine, and that process can end
 * while the page stays open. Without this, it ended silently: the layout stayed, the meetings
 * already fetched stayed on screen, and a note could be typed into for as long as the user liked
 * with nothing saved and nothing said. See `lib/reachable.ts` for what is watched.
 *
 * Not dismissible, and it does not replace the app. Hiding the interface would throw away the text
 * on screen, which at that moment is the only copy of anything unsaved — a user who can still see
 * it can still select it and put it somewhere safe.
 */
export function Unreachable() {
  const t = useT();
  const { handshake } = useEngine();
  const { reachable, check } = useReachable(handshake.port);
  if (reachable) return null;
  return (
    // `alert`, not `status`: the app is not doing what the user believes it is doing, and a screen
    // reader should say so at the point it becomes true rather than waiting to be asked.
    <p
      role="alert"
      data-testid="unreachable"
      className="border-rec/30 bg-rec-soft text-rec text-meta flex items-center gap-2 border-b px-4 py-2"
    >
      <PlugZap aria-hidden="true" className="size-4 shrink-0 stroke-[1.75]" />
      <span className="flex-1">
        {t("status.unreachable")}{" "}
        <span className="text-fg-dim">{t("status.unreachable_hint")}</span>
      </span>
      <button type="button" onClick={check} className="font-medium underline">
        {t("status.retry_now")}
      </button>
    </p>
  );
}
