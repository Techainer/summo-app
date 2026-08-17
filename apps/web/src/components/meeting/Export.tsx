import { Download, Languages } from "lucide-react";
import { useCallback, useMemo, useState } from "react";

import { useI18n } from "../../i18n/context";
import { cn } from "../../lib/cn";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import {
  ExportClient,
  FORMATS,
  download,
  fileName,
  type Format,
  type Translated,
} from "../../lib/export";
import { useLoad } from "../../lib/use-load";
import { Button, Card, CardBody, CardHeader } from "../ui";

/**
 * Taking a meeting out of Summo, and putting it into another language.
 *
 * Both were finished in the daemon and neither was reachable. `summo_vault::export` writes six
 * formats — the subtitle ones exist so a recording can be laid over a video, the plain one so a
 * transcript can be pasted into an email — and the route serving them had no caller. Translation
 * was worse: the transcript has rendered a translation since translations existed, so the half that
 * *shows* one was built, tested and shipped, while the half that *makes* one for a recording
 * captured without live translation could only be reached with `curl`.
 *
 * Sending somebody the part of a meeting they missed is one of the two things people do after a
 * meeting. It should not have been the thing this app could not do.
 */

/** Languages worth offering, and the daemon takes any tag — this is the shortlist, not the limit. */
const TARGETS = ["en", "vi", "ja", "zh"];

interface Props {
  meeting: string;
  title: string;
  /** `YYYY-MM-DD`, for the file name. */
  day: string;
  /** False for a typed note: there is no transcript to subtitle and nothing to translate. */
  recorded: boolean;
}

export function Export({ meeting, title, day, recorded }: Props) {
  const { t, locale } = useI18n();
  const { handshake } = useEngine();
  const say = useErrorText();
  const client = useMemo(() => new ExportClient(handshake), [handshake]);

  const [lang, setLang] = useState<string>("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<Translated | null>(null);

  const languages = useLoad(
    useCallback(async () => client.languages(meeting), [client, meeting]),
    [client, meeting],
  );
  const have = languages.data ?? [];

  const save = (format: Format) => {
    setBusy(format);
    setError(null);
    void (async () => {
      try {
        const text = await client.text(meeting, format, lang || undefined);
        download(fileName(title, day, format, lang || undefined), text);
      } catch (e) {
        setError(say(e));
      } finally {
        setBusy(null);
      }
    })();
  };

  const translate = (target: string) => {
    setBusy(target);
    setError(null);
    setDone(null);
    void (async () => {
      try {
        const outcome = await client.translate(meeting, target);
        setDone(outcome);
        setLang(target);
        languages.reload();
      } catch (e) {
        setError(say(e));
      } finally {
        setBusy(null);
      }
    })();
  };

  // The languages a name can be shown in, in the reader's own language: `en` reads as "Tiếng Anh"
  // to somebody using Summo in Vietnamese, and there is no reason to make them learn the tags.
  const nameOf = useMemo(() => {
    const names = new Intl.DisplayNames([locale], { type: "language" });
    return (code: string) => {
      try {
        return names.of(code) ?? code;
      } catch {
        return code;
      }
    };
  }, [locale]);

  return (
    <Card>
      <CardHeader title={t("export.title")} count={t("export.subtitle")} />
      <CardBody className="space-y-3">
        {/* Which language the file comes out in. Only shown once there is a choice — a meeting with
            no translation has exactly one answer and a row of one button is furniture. */}
        {recorded && have.length > 0 && (
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-fg-faint text-micro me-1">{t("export.language")}</span>
            <Choice on={lang === ""} onClick={() => setLang("")}>
              {t("export.original")}
            </Choice>
            {have.map((code) => (
              <Choice key={code} on={lang === code} onClick={() => setLang(code)}>
                {nameOf(code)}
              </Choice>
            ))}
          </div>
        )}

        <div className="flex flex-wrap items-center gap-1.5">
          <Download aria-hidden="true" className="text-fg-faint me-0.5 size-3.5" />
          {FORMATS.filter((format) => recorded || format === "md" || format === "txt").map(
            (format) => (
              <Button
                key={format}
                size="sm"
                variant="secondary"
                busy={busy === format}
                onClick={() => save(format)}
              >
                {format.toUpperCase()}
              </Button>
            ),
          )}
        </div>

        {recorded && (
          <div className="border-line flex flex-wrap items-center gap-1.5 border-t pt-3">
            <Languages aria-hidden="true" className="text-fg-faint me-0.5 size-3.5" />
            <span className="text-fg-faint text-micro me-1">{t("export.translate_into")}</span>
            {TARGETS.filter((code) => !have.includes(code)).map((code) => (
              <Button
                key={code}
                size="sm"
                variant="ghost"
                busy={busy === code}
                onClick={() => translate(code)}
              >
                {nameOf(code)}
              </Button>
            ))}
            {TARGETS.every((code) => have.includes(code)) && (
              <span className="text-fg-faint text-micro">{t("export.all_translated")}</span>
            )}
          </div>
        )}

        {done && (
          <p className="text-fg-dim text-meta">
            {done.complete
              ? t("export.translated", { count: done.translated, lang: nameOf(done.lang) })
              : t("export.translated_partly", { count: done.translated, missing: done.missing })}
          </p>
        )}

        {error && (
          <p role="alert" className="text-danger text-meta">
            {error}
          </p>
        )}
      </CardBody>
    </Card>
  );
}

function Choice({
  on,
  onClick,
  children,
}: {
  on: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      className={cn(
        "text-micro rounded-[var(--radius-pill)] border px-2.5 py-1 transition-colors",
        on ? "border-accent bg-accent-soft text-accent" : "border-line hover:border-fg-faint",
      )}
    >
      {children}
    </button>
  );
}
