import { useCallback, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";

import { useI18n } from "../../i18n/context";
import { CatalogueClient } from "../../lib/catalogue";
import { useEngine } from "../../lib/engine-context";
import { AUTO, autoAvailable, ordered, type Language } from "../../lib/languages";
import { url } from "../../lib/library";
import { useLoad } from "../../lib/use-load";
import { Select } from "../ui";

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
  languages,
  onChanged,
}: {
  /** What the daemon says it is decoding with, which is not always what this browser asked for. */
  live_model: string | undefined;
  spoken: string;
  into: string;
  languages: Language[];
  /** Re-read `/status`; the daemon answers before the pipeline is actually swapped. */
  onChanged: () => void;
}) {
  const { t, locale } = useI18n();
  const { handshake, retune, translate } = useEngine();
  const navigate = useNavigate();

  const catalogue = useLoad(
    useCallback(async () => new CatalogueClient(handshake).load(), [handshake]),
    [handshake],
  );

  const models = catalogue.data?.models ?? [];
  const speech = models.filter((m) => m.installed && m.task === "asr");
  const translators = models.filter((m) => m.installed && m.task === "translate");
  const chosen = speech.find((m) => m.id === live_model);

  const spokenOptions = chosen
    ? ordered(chosen.langs, locale)
    : ordered(
        languages.filter((l) => l.installed && l.model && !l.multilingual_only).map((l) => l.code),
        locale,
      );

  return (
    <div data-testid="listening-panel" className="border-accent/20 mt-2.5 border-t pt-3">
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        <Field label={t("settings.the_model")}>
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

        <Field label={t("record.spoken")}>
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

        <Field label={t("record.translate_live")}>
          <Select
            size="sm"
            aria-label={t("record.translate_live")}
            value={into}
            disabled={translators.length === 0}
            onChange={(event) => {
              translate(event.target.value);
              onChanged();
            }}
          >
            <option value="">{t("record.translate_off")}</option>
            {/* Every language the reader might want, not a shortlist: the translator is
                multilingual, and a fixed seven-entry list was the same mistake as the spoken one —
                a capability hidden behind an interface narrower than it. */}
            {ordered(TRANSLATABLE, locale).map((each) => (
              <option key={each.code} value={each.code}>
                {each.label}
              </option>
            ))}
          </Select>
        </Field>

        {translators.length > 1 && (
          <Field label={t("settings.mt_model")}>
            <Select
              size="sm"
              aria-label={t("settings.mt_model")}
              value={catalogue.data?.chosen?.translator ?? ""}
              onChange={(event) => {
                void pointTranslatorAt(handshake, event.target.value).finally(onChanged);
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
        {t("record.listening_note")} {translators.length === 0 && t("record.translate_needs_model")}{" "}
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
