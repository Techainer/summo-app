import { Suspense, lazy, useCallback, useEffect, useState, type ReactNode } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { url } from "../../lib/library";
import { AUTO, fetchLanguages, languageName, quality } from "../../lib/languages";
import { useLoad } from "../../lib/use-load";

/**
 * How often the running session is re-read while recording.
 *
 * Three seconds: fast enough that a change made elsewhere shows up before somebody wonders, slow
 * enough to be nothing next to the audio already going over the socket every 100 ms.
 */
const STATUS_POLL_MS = 3000;

/**
 * What the running recording is doing, and one control that opens everything it can be changed to.
 *
 * Shared by the two places a recording is visible, because there must be exactly one of it on
 * screen at a time:
 *
 * - `LiveBar`, on the meeting page, directly above the transcript — where somebody reading the
 *   words is looking when they notice the words are wrong.
 * - `ListeningIn`, in the app's chrome, on every *other* screen — because a recording survives
 *   navigation and "this is in English actually" is realised while looking at something else.
 *
 * Splitting it this way is the fix for two things at once. The chrome banner was the only way to
 * reach any of these settings, and it is drawn above the page rather than in it, so on the meeting
 * page the configuration for the meeting sat outside the meeting. It also had a dismiss button —
 * reasonable for a banner following you around the app, and on the meeting page it meant one click
 * permanently removed the only way to change the model, the language or the translation for the
 * rest of the recording.
 *
 * The panel itself is loaded on demand: this line is on screen for the whole of every recording and
 * the four dropdowns behind it are opened by a minority of them.
 */
const ListeningPanel = lazy(() =>
  import("./ListeningPanel").then((m) => ({ default: m.ListeningPanel })),
);

export function SessionControls({
  /** Buttons belonging to the surrounding chrome — going back to the meeting, dismissing. */
  extras,
  /** Set on the meeting page, where the bar around this is already the recording's own. */
  quiet = false,
  expanded = false,
}: {
  extras?: ReactNode;
  quiet?: boolean;
  /**
   * Start with the controls on screen rather than behind the link.
   *
   * True on the meeting page, which is the screen somebody is *on* while the call runs — the model,
   * the spoken language and the translation target are the whole reason to be looking at it, and
   * putting them one unlabelled click away meant a person hunting for them mid-call and reporting
   * that they had been removed. False in the banner the shell draws on every other screen, where
   * the same three dropdowns across the top of an unrelated page would be noise.
   */
  expanded?: boolean;
}) {
  const { handshake, session } = useEngine();
  const { t, locale } = useI18n();
  const [open, setOpen] = useState(expanded);
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
  // class of quiet mismatch this line exists to end.
  const live = useLoad(
    useCallback(async () => {
      const response = await fetch(url(handshake, "/status"));
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      // Tagged by `state`, flattened — `{"state":"recording","live_model":…}` — not an externally
      // tagged `{"Recording":{…}}`. Reading the wrong shape is how this spent its first run
      // insisting every meeting was being detected automatically.
      return (await response.json()) as {
        state?: string;
        live_model?: string;
        language?: string;
        translate_into?: string[];
      };
    }, [handshake]),
    [handshake, session.recording, generation],
  );

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
  const label = spoken === AUTO ? t("record.spoken_auto") : languageName(spoken, locale);
  const into = recording?.translate_into ?? [];

  // Kept in step with the daemon for as long as a recording is running.
  //
  // This used to re-read once, 600 ms after a change, on the assumption that the daemon applies one
  // quickly. Turning on translation disproves it: building SMALL100 takes about three seconds, so
  // the single re-read landed before anything had happened and nothing ever corrected it. The panel
  // then showed "Off" over a meeting that was translating every line, and the summary line above it
  // said nothing about translation at all — the exact silent mismatch this component exists to end.
  //
  // Polling rather than a longer timeout, because there is no delay that is right for both a warm
  // decoder swap and a cold 610 MB model, and because a change can also arrive from somewhere else
  // entirely: the settings screen, or a second window.
  useEffect(() => {
    if (!session.recording) return undefined;
    const timer = window.setInterval(() => setGeneration((n) => n + 1), STATUS_POLL_MS);
    return () => window.clearInterval(timer);
  }, [session.recording]);

  // Still nudged immediately after a change, so a fast one shows up at once rather than waiting out
  // the poll.
  const settle = () => window.setTimeout(() => setGeneration((n) => n + 1), 600);

  return (
    <>
      <div className="text-meta flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className={quiet ? "text-fg-faint text-micro" : "text-fg-dim"}>
          {t("record.listening_in", { language: label })}
          {recording?.live_model ? ` · ${recording.live_model}` : ""}
          {current && quality(current) === "poor" ? ` · ${t("record.spoken_poor")}` : ""}
          {/* Whether anything is being translated, which this could not say before. Silence here
              used to be indistinguishable from a translator that was quietly failing. */}
          {/* Every target, not the first one. A meeting subtitled into two languages that said it
              was subtitled into one is the same silence this line was added to end, one language
              further along. */}
          {into.length > 0
            ? ` · ${t("record.translating_into", {
                language: into.map((code) => languageName(code, locale)).join(", "),
              })}`
            : ""}
        </span>

        {extras}

        <button
          type="button"
          data-testid="listening-change"
          aria-expanded={open}
          onClick={() => setOpen((was) => !was)}
          className={
            quiet ? "text-accent text-micro font-medium underline" : "font-medium underline"
          }
        >
          {open ? t("common.done") : t("record.listening_change")}
        </button>
      </div>

      {open && (
        // No fallback: the chunk is small and served from the same daemon, and a spinner appearing
        // for one frame is more distracting than the panel simply arriving.
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
    </>
  );
}
