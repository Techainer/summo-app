import { useCallback, useState, type ReactNode } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";

import { useI18n } from "../../i18n/context";
import { CatalogueClient } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { url } from "../../lib/library";
import {
  AUTO,
  autoAvailable,
  fetchLanguages,
  languageName,
  ordered,
  quality,
} from "../../lib/languages";
import { useLoad } from "../../lib/use-load";
import { Select } from "../ui";

/**
 * What the recording is doing, said out loud while it does it — and every part of it changeable.
 *
 * A default is not enough, and this is the reason: a preference is right most of the time and wrong
 * exactly when it matters — the customer call in English, the standup that switched, the talk that
 * turned out to need subtitles. The old shape asked once, at install, and then never mentioned it
 * again, so a meeting recorded with the wrong settings looked identical to one recorded correctly
 * until somebody read the transcript.
 *
 * ## Why all four controls are here
 *
 * This banner used to offer exactly one: the spoken language, drawn from `/languages`, which lists
 * *languages the daemon can serve* rather than *what the current model can hear*. Three
 * consequences, all of which a user hit in one session and reported as one complaint:
 *
 * - **The model could not be changed at all.** `model_swap` has carried an `id` since it was
 *   written and nothing ever sent one.
 * - **A language the chosen model supports was not offered.** `/languages` reports the best model
 *   per language, so with Whisper pinned, Vietnamese — which ranks to Gipformer — was absent from a
 *   list of Whisper's own languages. Whisper hears Vietnamese; the list simply was not about
 *   Whisper. So the language list here comes from the model, and only falls back to `/languages`
 *   when no model is pinned and the daemon is picking per language.
 * - **Translation was invisible and immovable.** It was read once out of `session_start` and never
 *   looked at again, so it could not be turned on, off, retargeted, or even confirmed.
 *
 * ## What it does not do
 *
 * Only installed models are offered. Mid-meeting is the wrong moment to begin a 600 MB download,
 * and offering one would be offering to lose the next ten minutes of the call. The panel says so
 * and points at the models screen rather than pretending the choice is unavailable.
 *
 * Changing is not a restart: the daemon rebuilds the decoder under the running session and the
 * file, the timing and everything already transcribed continue. That is what makes this honest to
 * show at second three rather than as a question before the first word.
 *
 * It is deliberately not a modal. The promise is that pressing record records — anything that
 * stands between the press and the capture is a bug, including a dialog confirming what the user
 * already chose.
 */
export function ListeningIn() {
  const { session, retune, translate, handshake } = useEngine();
  const { t, locale } = useI18n();
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const [open, setOpen] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  // Bumped after a change, to re-read what the daemon now says rather than what this component
  // asked for — the two differ when a swap fails and the old pipeline keeps running.
  const [generation, setGeneration] = useState(0);

  const probe = useLoad(
    useCallback(async () => fetchLanguages(handshake), [handshake]),
    [handshake],
  );
  const languages = probe.data?.languages ?? [];

  // Opened lazily: a banner that appears on every screen during every recording should not fetch
  // the whole catalogue to draw one sentence. The panel behind "change" is what needs it.
  const catalogue = useLoad(
    useCallback(
      async () => (open ? new CatalogueClient(handshake).load() : null),
      // eslint-disable-next-line react-hooks/exhaustive-deps -- re-read after a write.
      [handshake, open, generation],
    ),
    [handshake, open, generation],
  );

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

  // Re-read `/status` after the daemon has had a moment to rebuild. It answers the command before
  // the pipeline is swapped, so an immediate read reports the state being replaced.
  const settle = () => window.setTimeout(() => setGeneration((n) => n + 1), 600);

  const models = catalogue.data?.models ?? [];
  const speech = models.filter((m) => m.installed && m.task === "asr");
  const translators = models.filter((m) => m.installed && m.task === "translate");
  const chosen = speech.find((m) => m.id === recording?.live_model);

  /**
   * The spoken languages worth offering, and where they come from.
   *
   * From the pinned model when there is one, because that is the question being asked: "what can
   * *this* model hear". From `/languages` otherwise, filtered to what is installed, because then
   * the daemon is choosing a model per language and an uninstalled one is a download rather than a
   * choice.
   */
  const spokenOptions = chosen
    ? ordered(chosen.langs, locale)
    : ordered(
        languages.filter((l) => l.installed && l.model && !l.multilingual_only).map((l) => l.code),
        locale,
      );

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
        <div data-testid="listening-panel" className="border-accent/20 mt-2.5 border-t pt-3">
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <Field label={t("settings.the_model")}>
              <Select
                size="sm"
                aria-label={t("settings.the_model")}
                value={recording?.live_model ?? ""}
                onChange={(event) => {
                  retune({ model: event.target.value });
                  settle();
                }}
              >
                {/* The running model, even when the catalogue has not arrived or no longer lists
                    it. A dropdown whose value is absent from its own options renders blank, and a
                    blank model field over a working recording reads as a broken recording. */}
                {recording?.live_model && !speech.some((m) => m.id === recording.live_model) && (
                  <option value={recording.live_model}>{recording.live_model}</option>
                )}
                {speech.map((each) => (
                  <option key={each.id} value={each.id}>
                    {each.name}
                  </option>
                ))}
              </Select>
            </Field>

            <Field label={t("record.spoken")}>
              <Select
                size="sm"
                aria-label={t("record.spoken")}
                value={spoken}
                onChange={(event) => {
                  retune({ language: event.target.value });
                  settle();
                }}
              >
                {(chosen ? chosen.langs.length > 1 : autoAvailable(languages)) && (
                  <option value={AUTO}>{t("record.spoken_auto")}</option>
                )}
                {spokenOptions.map((each) => (
                  <option key={each.code} value={each.code}>
                    {each.label}
                  </option>
                ))}
              </Select>
            </Field>

            <Field label={t("record.translate_live")}>
              <Select
                size="sm"
                aria-label={t("record.translate_live")}
                value={into}
                disabled={translators.length === 0}
                onChange={(event) => {
                  translate(event.target.value);
                  settle();
                }}
              >
                <option value="">{t("record.translate_off")}</option>
                {/* Every language the reader might want, not a shortlist: the translator is
                    multilingual, and a fixed seven-entry list was the same mistake as the spoken
                    one — a capability hidden behind an interface narrower than it. */}
                {ordered(TRANSLATABLE, locale).map((each) => (
                  <option key={each.code} value={each.code}>
                    {each.label}
                  </option>
                ))}
              </Select>
            </Field>

            {/* Only when there is a decision to make. One installed translator is not a choice, and
                a dropdown with one option in it is a control that exists to look busy. */}
            {translators.length > 1 && (
              <Field label={t("settings.mt_model")}>
                <Select
                  size="sm"
                  aria-label={t("settings.mt_model")}
                  value={catalogue.data?.chosen?.translator ?? ""}
                  onChange={(event) => {
                    void pointTranslatorAt(handshake, event.target.value).finally(settle);
                  }}
                >
                  {translators.map((each) => (
                    <option key={each.id} value={each.id}>
                      {each.name}
                    </option>
                  ))}
                </Select>
              </Field>
            )}
          </div>

          <p className="text-fg-faint text-micro mt-2.5">
            {t("record.listening_note")}{" "}
            {translators.length === 0 && t("record.translate_needs_model")}{" "}
            <button
              type="button"
              onClick={() => void navigate({ to: "/models" })}
              className="underline"
            >
              {t("record.manage_models")}
            </button>
          </p>
        </div>
      )}
    </div>
  );
}

/** A label above a control, sized for a banner rather than a settings page. */
function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block">
      <span className="text-fg-faint text-micro">{label}</span>
      <span className="mt-1 block">{children}</span>
    </label>
  );
}

/**
 * Point the translator role at a model, mid-meeting.
 *
 * Over HTTP rather than the socket because it is a *setting* — the next meeting starts with it too
 * — and because the socket's `translate` command is about the target language, not about which
 * model does the work. The change reaches the running session on the next line, since the live
 * translator is rebuilt from settings whenever the target changes.
 */
async function pointTranslatorAt(
  handshake: { port: number; token: string },
  id: string,
): Promise<void> {
  await fetch(url(handshake, "/settings/models"), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ role: "translator", model: id }),
  });
}

/**
 * What live translation can target.
 *
 * The languages `small100` and `m2m100` are trained on, which is the same hundred either way. Kept
 * as codes rather than a labelled table so the names come from `Intl.DisplayNames` in whichever
 * language the reader is using — a hand-written table would be a hundred entries per locale that
 * nobody would keep correct.
 */
const TRANSLATABLE = [
  "af",
  "am",
  "ar",
  "ast",
  "az",
  "ba",
  "be",
  "bg",
  "bn",
  "br",
  "bs",
  "ca",
  "ceb",
  "cs",
  "cy",
  "da",
  "de",
  "el",
  "en",
  "es",
  "et",
  "fa",
  "ff",
  "fi",
  "fr",
  "fy",
  "ga",
  "gd",
  "gl",
  "gu",
  "ha",
  "he",
  "hi",
  "hr",
  "ht",
  "hu",
  "hy",
  "id",
  "ig",
  "ilo",
  "is",
  "it",
  "ja",
  "jv",
  "ka",
  "kk",
  "km",
  "kn",
  "ko",
  "lb",
  "lg",
  "ln",
  "lo",
  "lt",
  "lv",
  "mg",
  "mk",
  "ml",
  "mn",
  "mr",
  "ms",
  "my",
  "ne",
  "nl",
  "no",
  "ns",
  "oc",
  "or",
  "pa",
  "pl",
  "ps",
  "pt",
  "ro",
  "ru",
  "sd",
  "si",
  "sk",
  "sl",
  "so",
  "sq",
  "sr",
  "ss",
  "su",
  "sv",
  "sw",
  "ta",
  "th",
  "tl",
  "tn",
  "tr",
  "uk",
  "ur",
  "uz",
  "vi",
  "wo",
  "xh",
  "yi",
  "yo",
  "zh",
  "zu",
];
