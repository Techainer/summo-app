import { useCallback, useState, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { CatalogueClient } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { readJson, useErrorText } from "../../lib/errors";
import { AUTO, autoAvailable, languageName, ordered, type Language } from "../../lib/languages";
import { url } from "../../lib/library";
import { fetchPlan } from "../../lib/plan";
import { useLoad } from "../../lib/use-load";
import { Select } from "../ui";
import { TranslateTargets } from "./TranslateTargets";

/**
 * Everything about a running recording that can still be changed, in four controls.
 *
 * Split out of `ListeningIn` and loaded on demand, for a reason the bundle budget made concrete:
 * the banner renders on every screen for the whole of every recording, and this panel renders only
 * after somebody presses "change". Keeping the catalogue client, the hundred-entry language table
 * and four dropdowns in the first load cost 200.1 kB against a 200 kB budget — for a panel most
 * recordings never open.
 *
 * ## What each control is for
 *
 * **The model.** `model_swap` has carried an `id` since it was written and nothing ever sent one,
 * so the model was the one thing about a meeting that could not be corrected once it started.
 *
 * **The spoken language, taken from the model.** `/languages` answers "what would serve this
 * language", ranking each language to its best model — so with Whisper pinned, Vietnamese ranks to
 * Gipformer and vanished from a list of Whisper's own languages. Whisper hears Vietnamese. The list
 * was not about Whisper. When no model is pinned the daemon *is* choosing per language, and then
 * `/languages` is the right source and this falls back to it.
 *
 * **The translation target**, including off. It was read once out of `session_start` and never
 * again, so a call that turned out not to need subtitles paid for a translator on every line until
 * it ended.
 *
 * **Which translator**, but only when more than one is installed. A dropdown with one option is a
 * control that exists to look busy.
 *
 * Only installed models are offered. Mid-meeting is the wrong moment to start a 600 MB download,
 * and offering one would be offering to lose the next ten minutes of the call — so the panel says
 * what is missing and points at the models screen instead of pretending the choice is unavailable.
 */
export function ListeningPanel({
  live_model,
  spoken,
  into,
  refine,
  languages,
  onChanged,
}: {
  /** What the daemon says it is decoding with, which is not always what this browser asked for. */
  live_model: string | undefined;
  spoken: string;
  into: string[];
  /** The second speech model the daemon says it is checking the text with, or `""` for none. */
  refine: string;
  languages: Language[];
  /** Re-read `/status`; the daemon answers before the pipeline is actually swapped. */
  onChanged: () => void;
}) {
  const { t, locale } = useI18n();
  const { handshake, retune, translate, refine: setRefine, transcript } = useEngine();
  const navigate = useNavigate();
  const say = useErrorText();
  // A refused translator swap, said out loud. See `pointTranslatorAt`.
  const [swapFailed, setSwapFailed] = useState<string | null>(null);

  const catalogue = useLoad(
    useCallback(async () => new CatalogueClient(handshake).load(), [handshake]),
    [handshake],
  );

  // What the daemon would actually translate *with*, which the catalogue cannot answer: an
  // installed translation model and a configured one are different facts, and the daemon resolves
  // between them. Asked here so this panel offers translation only when it would work.
  const plan = useLoad(
    useCallback(async () => fetchPlan(handshake), [handshake]),
    [handshake],
  );

  const models = catalogue.data?.models ?? [];
  const speech = models.filter((m) => m.installed && m.task === "asr");
  const translators = models.filter((m) => m.installed && m.task === "translate");
  const chosen = speech.find((m) => m.id === live_model);

  // The model that will do it, named. `using` is the daemon's own answer; the catalogue supplies
  // the readable name for it.
  const using = plan.data?.translation.using ?? null;
  const usingName = translators.find((m) => m.id === using)?.name ?? using;
  // An endpoint is a translator too, and one this panel cannot see in the catalogue. Offering the
  // target languages only when a *file* is installed would have refused the one configuration that
  // has always worked.
  const endpoint =
    plan.data?.translation.local === false && plan.data.translation.provider !== null;
  // Unknown until the daemon has answered, and unknown is not "no". A panel opened while the plan
  // is still in flight used to render the translation control greyed out under the words "no
  // translation model is installed" — on a machine with one installed — and then quietly come
  // right a moment later. Whoever read it in that moment had been told something false.
  const known = plan.data !== null;
  const canTranslate = !known || using !== null || endpoint;

  const spokenOptions = chosen
    ? ordered(chosen.langs, locale)
    : ordered(
        languages.filter((l) => l.installed && l.model && !l.multilingual_only).map((l) => l.code),
        locale,
      );

  return (
    <div data-testid="listening-panel" className="border-accent/20 mt-2.5 border-t pt-3">
      {/* Sized by how often each is reached for, not by how long its longest option is.
          
          An even four-column grid gave `Whisper tiny (99 languages)` the same width as `Tắt`, and
          the model is the control somebody touches once a month while the translation target is the
          one they touch during the call. Reported as "the most important thing is tiny and the model
          picker is far too big".
          
          So: translation takes two columns of six and comes first on a narrow screen; the two model
          pickers share the rest and truncate, which is what a name nobody is reading should do. */}
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-6">
        <Field label={t("settings.the_model")} className="lg:order-2 lg:col-span-2">
          <Select
            size="sm"
            aria-label={t("settings.the_model")}
            value={live_model ?? ""}
            onChange={(event) => {
              retune({ model: event.target.value });
              onChanged();
            }}
          >
            {/* The running model, even when the catalogue has not arrived or no longer lists it. A
                dropdown whose value is absent from its own options renders blank, and a blank model
                field over a working recording reads as a broken recording. */}
            {live_model && !speech.some((m) => m.id === live_model) && (
              <option value={live_model}>{live_model}</option>
            )}
            {speech.map((each) => (
              <option key={each.id} value={each.id}>
                {each.name}
              </option>
            ))}
          </Select>
        </Field>

        <Field label={t("record.spoken")} className="lg:order-3">
          <Select
            size="sm"
            aria-label={t("record.spoken")}
            value={spoken}
            onChange={(event) => {
              retune({ language: event.target.value });
              onChanged();
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

        {/* The second speech model, live.
            
            Shipped without a control at all: it could be chosen on the models screen and nowhere
            else, so the one decision you make *because of what you are hearing* — this call turned
            out to be half in English — could only be made before the call started. Recognition gets
            the same treatment as translation, which is the whole point of a panel that changes a
            meeting while it runs. */}
        {speech.length > 1 && (
          <Field label={t("settings.mt_second")} className="lg:order-4">
            <Select
              size="sm"
              aria-label={t("settings.mt_second")}
              value={refine}
              onChange={(event) => {
                setRefine(event.target.value);
                onChanged();
              }}
            >
              <option value="">{t("record.translate_off")}</option>
              {speech
                .filter((m) => m.id !== live_model)
                .map((each) => (
                  <option key={each.id} value={each.id}>
                    {each.name}
                  </option>
                ))}
            </Select>
          </Field>
        )}

        <Field
          label={t("record.translate_live")}
          className="sm:col-span-2 lg:order-1 lg:col-span-2"
        >
          {/* Every language the reader might want, not a shortlist: the translator is multilingual,
              and a fixed seven-entry list was the same mistake as the spoken one — a capability
              hidden behind an interface narrower than it. More than one at a time for the same
              reason: the model is already loaded, and a call can have two readers. */}
          <TranslateTargets
            value={into}
            options={ordered(TRANSLATABLE, locale)}
            disabled={!canTranslate}
            onChange={(next) => {
              translate(next);
              onChanged();
            }}
          />
        </Field>

        {translators.length > 1 && (
          <Field label={t("settings.mt_model")} className="lg:order-5">
            <Select
              size="sm"
              aria-label={t("settings.mt_model")}
              value={using ?? ""}
              onChange={(event) => {
                setSwapFailed(null);
                void pointTranslatorAt(handshake, event.target.value)
                  .then(() => {
                    // `using` is the daemon's answer, not this dropdown's — so the control only
                    // moves once the daemon has been re-asked.
                    plan.reload();
                    onChanged();
                  })
                  .catch((error: unknown) => setSwapFailed(say(error)));
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

      {/* Turned on, and nothing translated yet.
          
          Lines already finished keep whatever they were given — turning translation on does not go
          back over the transcript, which is deliberate and documented on `Command::Translate`. The
          consequence was not: somebody who turns it on a minute into a call sees the banner say
          `đang dịch sang Tiếng Việt` over a transcript with no translation anywhere in it, and the
          honest conclusion from that screen is that the feature is broken. It is not — it is
          waiting for the next sentence, and SMALL100 spends several seconds loading before even
          that one. Reported exactly that way, twice.
          
          Only while the wait is real: the moment any line has a subtitle, this has nothing to say. */}
      {into.length > 0 && !transcript.segments.some((segment) => segment.translation) && (
        <p className="text-fg-dim text-micro mt-2">{t("record.translate_pending")}</p>
      )}

      {/* Translating into the language being spoken, which is the one target that cannot show you
          anything. `Tiếng Việt` is pinned first in that list — it is the reader's own language and
          for every *other* language control that is the right place for it — so on a Vietnamese
          call it is also the easiest entry to hit by accident. The daemon dutifully translates
          Vietnamese to Vietnamese, the subtitle is the sentence again, and the panel says
          `đang dịch sang Tiếng Việt` over a transcript with no visible translation in it.

          Said, not prevented: a meeting held in English by a Vietnamese speaker who set the spoken
          language wrong has a real reason to want this, and refusing the choice would be guessing
          which of the two settings is the mistake. */}
      {spoken !== AUTO && into.includes(spoken) && (
        <p className="text-fg-dim text-micro mt-2">
          {t("record.translate_same", { language: languageName(spoken, locale) })}
        </p>
      )}

      {/* A swap the daemon refused. Without this the dropdown showed the new model and the meeting
          kept translating with the old one — the same silent disagreement between a control and the
          pipeline behind it that `in-meeting.mjs` checks `/status` to catch for the speech model. */}
      {swapFailed && <p className="text-rec text-micro mt-2">{swapFailed}</p>}

      <p className="text-fg-faint text-micro mt-2.5">
        {t("record.listening_note")}{" "}
        {/* Which model translates, when there is no dropdown saying so. One installed translator
            needs no chooser, but it still has a name — and leaving it unsaid is what let the panel
            offer translation that silently resolved to an endpoint nobody was running. */}
        {canTranslate && translators.length < 2 && usingName
          ? `${t("record.translate_with", { model: usingName })} `
          : ""}
        {!canTranslate && `${t("record.translate_needs_model")} `}
        <button
          type="button"
          onClick={() => void navigate({ to: "/models" })}
          className="underline"
        >
          {t("record.manage_models")}
        </button>
      </p>
    </div>
  );
}

/** A label above a control, sized for a banner rather than a settings page. */
function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={cn("block min-w-0", className)}>
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
 *
 * The answer is read rather than discarded. This was `await fetch(...)` with the response thrown
 * away and `.finally(onChanged)` at the call site, so a refusal — an id the daemon does not have, a
 * model whose task is not translation, a write to a read-only settings file — left the dropdown
 * showing a model that was not translating anything.
 */
async function pointTranslatorAt(
  handshake: { port: number; token: string },
  id: string,
): Promise<void> {
  await readJson<unknown>(
    await fetch(url(handshake, "/settings/models"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ role: "translator", model: id }),
    }),
  );
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
