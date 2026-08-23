import { Suspense, lazy, useCallback, useState } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { url } from "../../lib/library";
import { AUTO, fetchLanguages, languageName, quality } from "../../lib/languages";
import { useLoad } from "../../lib/use-load";

/**
 * What the recording is doing, said out loud while it does it.
 *
 * A default is not enough, and this is the reason: a preference is right most of the time and wrong
 * exactly when it matters — the customer call in English, the standup that switched, the talk that
 * turned out to need subtitles. The old shape asked once, at install, and then never mentioned it
 * again, so a meeting recorded with the wrong settings looked identical to one recorded correctly
 * until somebody read the transcript.
 *
 * So while recording, the app says what it is hearing and what it is translating into, and offers
 * to change either. Changing is not a restart: the daemon rebuilds only what changed under the
 * running session, and the file, the timing and everything already transcribed continue. That is
 * what makes this honest to show at second three rather than as a question before the first word.
 *
 * It is deliberately not a modal. The promise is that pressing record records — anything that
 * stands between the press and the capture is a bug, including a dialog confirming what the user
 * already chose.
 *
 * The controls themselves are in `ListeningPanel`, loaded when somebody asks for them. This banner
 * is on screen for the whole of every recording on every screen; the panel is opened by a minority
 * of them, and it carries the catalogue client and a hundred-entry language table.
 */
const ListeningPanel = lazy(() =>
  import("./ListeningPanel").then((m) => ({ default: m.ListeningPanel })),
);

export function ListeningIn() {
  const { session, handshake } = useEngine();
  const { t, locale } = useI18n();
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [open, setOpen] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  // Bumped after a change, to re-read what the daemon now says rather than what the panel asked
  // for — the two differ when a swap fails and the old pipeline keeps running.
  const [generation, setGeneration] = useState(0);

  const probe = useLoad(
    useCallback(async () => fetchLanguages(handshake), [handshake]),
    [handshake],
  );
  const languages = probe.data?.languages ?? [];

  // What the *daemon* resolved, not what this browser last chose. A session started without naming
  // a language falls back to the settings file, and a banner reading the local preference would
  // announce "detecting automatically" while the daemon confidently decoded Vietnamese — the exact
  // class of quiet mismatch this banner exists to end. Re-read after every change, and while
  // recording, because a swap makes the previous answer wrong.
  const live = useLoad(
    useCallback(async () => {
      const response = await fetch(url(handshake, "/status"));
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      // Tagged by `state`, flattened — `{"state":"recording","live_model":…}` — not an
      // externally tagged `{"Recording":{…}}`. Reading the wrong shape is how this banner spent its
      // first run insisting every meeting was being detected automatically.
      return (await response.json()) as {
        state?: string;
        live_model?: string;
        language?: string;
        translate_to?: string;
      };
    }, [handshake]),
    [handshake, session.recording, generation],
  );

  if (!session.recording || dismissed) return null;

  const recording = live.data?.state === "recording" ? live.data : undefined;

  // A session that named no language is not "detecting" — it is running whatever the model does.
  // For a single-language model that is its language, and saying "automatic" there would be the
  // same lie in the other direction: gipformer hears Vietnamese and nothing else, whatever the
  // session forgot to specify.
  const named = recording?.language;
  const covered = languages.filter(
    (language) => language.model === recording?.live_model && !language.multilingual_only,
  );
  const spoken = named ?? (covered.length === 1 ? (covered[0]?.code ?? AUTO) : AUTO);
  const current = languages.find((language) => language.code === spoken);
  const auto = spoken === AUTO;
  const label = auto ? t("record.spoken_auto") : languageName(spoken, locale);
  const into = recording?.translate_to ?? "";

  // The daemon answers a change before the pipeline is actually swapped, so an immediate read
  // reports the state being replaced.
  const settle = () => window.setTimeout(() => setGeneration((n) => n + 1), 600);

  return (
    <div className="border-accent/30 bg-accent-soft rounded-[var(--radius-card)] border px-3 py-2">
      <div className="text-meta flex flex-wrap items-center gap-2">
        <span className="text-fg-dim">
          {t("record.listening_in", { language: label })}
          {recording?.live_model ? ` · ${recording.live_model}` : ""}
          {current && quality(current) === "poor" ? ` · ${t("record.spoken_poor")}` : ""}
          {/* Whether anything is being translated, which the banner could not say before. Silence
              here used to be indistinguishable from a translator that was quietly failing. */}
          {into
            ? ` · ${t("record.translating_into", { language: languageName(into, locale) })}`
            : ""}
        </span>

        {/* Back to the meeting. A recording survives navigation, so somebody who wandered off to
            the library while it ran had the red clock in the header — which stops the recording —
            and no way at all to return to the page the words are landing on. */}
        {session.meeting && !pathname.startsWith(`/pages/${session.meeting}`) && (
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
          data-testid="listening-change"
          aria-expanded={open}
          onClick={() => setOpen((was) => !was)}
          className="font-medium underline"
        >
          {open ? t("common.done") : t("record.listening_change")}
        </button>
        <button
          type="button"
          onClick={() => setDismissed(true)}
          className="text-fg-faint ms-auto"
          aria-label={t("common.dismiss")}
        >
          <X aria-hidden="true" className="size-3.5" />
        </button>
      </div>

      {open && (
        // No fallback: the chunk is small and local, and a spinner appearing under a banner for
        // one frame is more distracting than the panel simply arriving.
        <Suspense fallback={null}>
          <ListeningPanel
            live_model={recording?.live_model}
            spoken={spoken}
            into={into}
            languages={languages}
            onChanged={settle}
          />
        </Suspense>
      )}
    </div>
  );
}
