import { useState } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";

import { SessionControls } from "./SessionControls";
import { useT } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";

/**
 * What the running recording is hearing, on every screen except the one it is being written into.
 *
 * A default is not enough, and this is the reason: a preference is right most of the time and wrong
 * exactly when it matters — the customer call in English, the standup that switched, the talk that
 * turned out to need subtitles. The old shape asked once, at install, and then never mentioned it
 * again, so a meeting recorded with the wrong settings looked identical to one recorded correctly
 * until somebody read the transcript.
 *
 * ## Why it stops at the meeting page
 *
 * This is drawn by the shell, above the page, so on the meeting page the settings *for the meeting*
 * sat outside the meeting — in the chrome, next to the nudges, above a transcript they controlled.
 * `LiveBar` now carries the same control inside the meeting, directly over the words, and this
 * yields to it.
 *
 * Yielding rather than both rendering, because there must be exactly one: two controls named "Đổi"
 * on one screen is an ambiguity for anybody reading it and a broken selector for the end-to-end
 * suite. That is not hypothetical — an earlier attempt at this rendered both, and `full-flow.mjs`
 * caught it.
 *
 * Dismissable, and only here. Following somebody around the app is what earns a banner an ✕; the
 * meeting page's copy has none, because dismissing it there would remove the only way to change the
 * model, the language or the translation for the rest of the recording.
 */
export function ListeningIn() {
  const { session } = useEngine();
  const t = useT();
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [dismissed, setDismissed] = useState(false);

  const onMeeting = session.meeting !== null && pathname.startsWith(`/pages/${session.meeting}`);
  if (!session.recording || dismissed || onMeeting) return null;

  return (
    <div className="border-accent/30 bg-accent-soft rounded-[var(--radius-card)] border px-3 py-2">
      <SessionControls
        extras={
          <>
            {/* Back to the meeting. A recording survives navigation, so somebody who wandered off
                to the library while it ran had the red clock in the header — which stops the
                recording — and no way at all to return to the page the words are landing on. */}
            {session.meeting && (
              <button
                type="button"
                data-testid="back-to-meeting"
                onClick={() =>
                  void navigate({
                    to: "/pages/$pageId",
                    params: { pageId: session.meeting as string },
                  })
                }
                className="text-accent font-medium underline"
              >
                {t("record.open_meeting")}
              </button>
            )}
            <button
              type="button"
              onClick={() => setDismissed(true)}
              className="text-fg-faint order-last ms-auto"
              aria-label={t("common.dismiss")}
            >
              <X aria-hidden="true" className="size-3.5" />
            </button>
          </>
        }
      />
    </div>
  );
}
