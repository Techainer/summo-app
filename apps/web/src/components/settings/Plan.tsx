import { useNavigate } from "@tanstack/react-router";
import { AlertTriangle, AudioLines, Cpu, Languages, Sparkles, Users } from "lucide-react";
import { useCallback } from "react";

import { useI18n, useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { fetchPlan } from "../../lib/plan";
import { useLoad } from "../../lib/use-load";
import { Card, CardBody, CardHeader } from "../ui";

/**
 * Which model does what, on one card.
 *
 * Three separate settings decide three separate jobs and the interface never put them together:
 * recognition is a model from Summo's own registry, running here; translation is either a model
 * inside the app or a provider; summaries and answers are a language model that is usually somebody
 * else's server. Somebody asking "which model am I using" had to open three screens and know that
 * the question had three answers.
 *
 * The important row is the first, and the important thing about it is whether the model can serve
 * the language that was chosen. It could not, silently: choosing a spoken language wrote the
 * language and left the model alone, so a Vietnamese-only model went on decoding a Japanese meeting.
 * The daemon repoints it now; this is where a person can see that it did.
 */
export function Plan() {
  const { t } = useI18n();
  const { handshake } = useEngine();
  const plan = useLoad(
    useCallback(async () => fetchPlan(handshake), [handshake]),
    [handshake],
  );

  const data = plan.data;
  if (!data) return null;

  const wrong = data.speech.model !== null && !data.speech.covers_language;
  const missing = data.speech.model === null;

  return (
    <Card>
      <CardHeader title={t("plan.title")} count={t("plan.subtitle")} />
      <CardBody className="space-y-2.5">
        <Row
          icon={Cpu}
          label={t("plan.speech")}
          value={data.speech.name ?? data.speech.model ?? t("plan.none")}
          note={t("plan.on_device")}
          bad={wrong || missing}
        />

        {(wrong || missing) && (
          <p className="border-blocked/30 bg-blocked-soft text-blocked text-meta flex items-start gap-2 rounded-[var(--radius-card)] border px-3 py-2">
            <AlertTriangle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
            <span>{missing ? t("plan.no_model") : t("plan.wrong_language")}</span>
          </p>
        )}

        {/* Something better that is *already installed*, so it is one click rather than a download.
            A recommendation to fetch 300 MB belongs on the models screen, not in a settings row. */}
        {data.speech.better && (
          <p className="text-fg-dim text-meta">
            {t("plan.better", {
              name: data.speech.better.name,
              accuracy: Math.round(data.speech.better.accuracy * 100),
            })}
          </p>
        )}

        {/* The two nothing asked about until they were missing. A recording needs the detector; a
            transcript that names speakers needs the fingerprint. Both are one click from here when
            they are not there — the alternative is finding out mid-meeting. */}
        <Row
          icon={AudioLines}
          label={t("plan.detector")}
          value={data.detector.installed ? t("plan.ready") : t("plan.missing")}
          note={data.detector.installed ? t("plan.on_device") : undefined}
          bad={!data.detector.installed}
          action={data.detector.installed ? undefined : { id: data.detector.id }}
        />

        <Row
          icon={Users}
          label={t("plan.speakers")}
          value={data.speakers.installed ? t("plan.ready") : t("plan.missing")}
          note={data.speakers.installed ? t("plan.on_device") : undefined}
          action={data.speakers.installed ? undefined : { id: data.speakers.id }}
        />

        <Row
          icon={Languages}
          label={t("plan.translation")}
          value={
            data.translation.local
              ? t("plan.in_app")
              : (data.translation.model ?? data.translation.provider ?? t("plan.same_as_below"))
          }
          note={data.translation.local ? t("plan.on_device") : t("plan.over_network")}
        />

        <Row
          icon={Sparkles}
          label={t("plan.summaries")}
          value={data.language_model.model ?? t("plan.none")}
          note={data.language_model.provider}
        />

        {/* No footnote explaining that the table is derived. It is a table of what the app is
            using; how it knows is not the reader's problem. */}
      </CardBody>
    </Card>
  );
}

function Row({
  icon: Icon,
  label,
  value,
  note,
  bad = false,
  action,
}: {
  icon: typeof Cpu;
  label: string;
  value: string;
  note?: string | null;
  bad?: boolean;
  /** A model this row is missing, offered as a one-click install. */
  action?: { id: string };
}) {
  const t = useT();
  const navigate = useNavigate();

  return (
    <div className="flex items-baseline gap-3">
      <Icon aria-hidden="true" className="text-fg-faint size-3.5 shrink-0 translate-y-0.5" />
      <span className="text-fg-dim text-meta w-32 shrink-0">{label}</span>
      <span className={cn("min-w-0 flex-1 truncate font-medium", bad && "text-blocked")}>
        {value}
      </span>
      {action && (
        <button
          type="button"
          onClick={() => void navigate({ to: "/models", search: {} })}
          className="text-accent text-micro shrink-0 font-medium underline"
        >
          {t("setup.install")}
        </button>
      )}
      {note && <span className="text-fg-faint text-micro shrink-0">{note}</span>}
    </div>
  );
}
