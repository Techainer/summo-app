import { useCallback } from "react";

import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { readyNow, warmUp } from "../../lib/languages";
import { useLoad } from "../../lib/use-load";

/**
 * Whether the next recording will start instantly, and making it so.
 *
 * Building a decoder costs about three and a half seconds — measured, on the released build — and
 * until now that was paid at the start of every meeting, after the button was pressed, with nothing
 * on screen to say why the transcript had not begun. The daemon can build one ahead of time; this
 * asks it to, when the card is opened and nothing is recording.
 *
 * It says so quietly. A user who never notices this line is the point: it exists so that pressing
 * record produces text immediately, not so that anyone reads about model loading.
 */
export function WarmUp() {
  const { handshake, session } = useEngine();
  const t = useT();

  const state = useLoad(
    useCallback(async () => {
      // What is loaded already, then — only if nothing is — the request to load one. Asking first
      // keeps a reopened card from rebuilding a decoder that is already sitting there.
      const already = await readyNow(handshake);
      // `undefined` is "this build cannot warm anything" — do not ask it to.
      if (already === undefined || already || session.recording) return already ?? null;
      return await warmUp(handshake);
    }, [handshake, session.recording]),
    [handshake, session.recording],
  );

  if (session.recording || !state.data) return null;

  return (
    <p className="text-fg-faint text-micro mb-2">
      {t("record.warm_ready", { model: state.data.model })}
    </p>
  );
}
