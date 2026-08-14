import { useCallback, useState } from "react";

import { useI18n } from "../../i18n/context";
import { useEngine } from "../../lib/engine-context";
import { url } from "../../lib/library";
import {
  AUTO,
  autoAvailable,
  fetchLanguages,
  languageName,
  quality,
  ready,
} from "../../lib/languages";
import { useLoad } from "../../lib/use-load";

/**
 * What the recording is listening for, said out loud while it records.
 *
 * A default is not enough, and this is the reason: a language preference is right most of the time
 * and wrong exactly when it matters — the customer call in English, the standup that switched. The
 * old shape asked once, at install, and then never mentioned it again, so a meeting recorded in the
 * wrong language looked identical to one recorded in the right one until somebody read the
 * transcript.
 *
 * So while recording, the app says which language it is hearing and offers to change it. Changing
 * is not a restart: the daemon rebuilds the decoder under the running session, and the file, the
 * timing and everything already transcribed continue. That is what makes this honest to show at
 * second three rather than as a question before the first word.
 *
 * It is deliberately not a modal. The promise is that pressing record records — anything that
 * stands between the press and the capture is a bug, including a dialog asking to confirm what the
 * user already chose.
 */
export function ListeningIn() {
  const { session, retune } = useEngine();
  const { t, locale } = useI18n();
  const [open, setOpen] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  // Bumped after a change, to re-read what the daemon now says rather than what this component
  // asked for — the two differ when a swap fails and the old pipeline keeps running.
  const [generation, setGeneration] = useState(0);
  const { handshake } = useEngine();

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

  // Only languages that can be used right now. Mid-meeting is the wrong moment to start a 153 MB
  // download, and offering one would be offering to lose the next four minutes of the call.
  const usable = languages.filter((language) => ready(language));
  const canAuto = autoAvailable(languages);

  return (
    <div className="border-accent/30 bg-accent-soft text-meta flex flex-wrap items-center gap-2 rounded-[var(--radius-card)] border px-3 py-2">
      <span className="text-fg-dim">
        {t("record.listening_in", { language: label })}
        {recording?.live_model ? ` · ${recording.live_model}` : ""}
        {current && quality(current) === "poor" ? ` · ${t("record.spoken_poor")}` : ""}
      </span>

      {!open ? (
        <>
          <button type="button" onClick={() => setOpen(true)} className="font-medium underline">
            {t("record.listening_change")}
          </button>
          <button
            type="button"
            onClick={() => setDismissed(true)}
            className="text-fg-faint ms-auto"
            aria-label={t("common.dismiss")}
          >
            ✕
          </button>
        </>
      ) : (
        <label className="flex items-center gap-2">
          <span className="sr-only">{t("record.spoken")}</span>
          <select
            aria-label={t("record.spoken")}
            value={spoken}
            onChange={(event) => {
              retune(event.target.value);
              setOpen(false);
              // The daemon rebuilds the decoder before it answers; a moment later `/status` is the
              // truth about whether it worked.
              window.setTimeout(() => setGeneration((n) => n + 1), 600);
            }}
            className="border-line bg-bg-soft text-fg h-7 rounded-[var(--radius-card)] border px-2 text-sm"
          >
            {canAuto && <option value={AUTO}>{t("record.spoken_auto")}</option>}
            {usable.map((language) => (
              <option key={language.code} value={language.code}>
                {languageName(language.code, locale)}
              </option>
            ))}
          </select>
          <span className="text-fg-faint text-micro">{t("record.listening_note")}</span>
        </label>
      )}
    </div>
  );
}
