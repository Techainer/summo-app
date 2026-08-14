import { Mail } from "lucide-react";
import { useState } from "react";

import { useI18n } from "../../i18n/context";
import {
  ComposeClient,
  copy,
  mailto,
  type Composed,
  type Kind,
  type Tone,
} from "../../lib/compose";
import { useEngine } from "../../lib/engine-context";
import { useErrorText } from "../../lib/errors";
import { Button, Input, Labelled, SegmentedControl, TextArea } from "../ui";

const KINDS: Kind[] = ["email", "message", "recap", "actions"];
const TONES: Tone[] = ["neutral", "friendly", "formal"];

/**
 * Write the follow-up, out of the meeting that is already on screen.
 *
 * Four shapes rather than a prompt box, because the shape is the part a model gets wrong: an email
 * needs a subject and a sign-off, a chat message must fit in a glance, a recap cannot say "as
 * discussed" to people who were not there, and a list of actions is a list.
 *
 * Everything it produces is editable before it goes anywhere, and it goes nowhere by itself: the
 * buttons are copy, open in your own mail app, and keep as a note. That is deliberate. A model
 * writing a customer email will occasionally invent a deadline — the prompt makes it mark gaps with
 * `[…]` instead, but the real defence is that a person reads it and presses send themselves.
 */
export function ComposePanel({ meeting, title }: { meeting: string; title: string }) {
  const { handshake } = useEngine();
  const { t } = useI18n();
  const say = useErrorText();
  const client = new ComposeClient(handshake);

  const [open, setOpen] = useState(false);
  const [kind, setKind] = useState<Kind>("email");
  const [tone, setTone] = useState<Tone>("neutral");
  const [audience, setAudience] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState<Composed | null>(null);
  const [subject, setSubject] = useState("");
  const [body, setBody] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const run = async () => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      const composed = await client.compose(meeting, {
        kind,
        tone,
        audience: audience.trim() || undefined,
      });
      setDraft(composed);
      setSubject(composed.subject ?? "");
      setBody(composed.body);
    } catch (e) {
      setError(say(e));
    } finally {
      setBusy(false);
    }
  };

  const keep = async () => {
    setError(null);
    try {
      setNote(await client.save(meeting, subject.trim() || title, body));
    } catch (e) {
      setError(say(e));
    }
  };

  const wholeMessage = subject.trim() ? `${subject}\n\n${body}` : body;

  return (
    <section
      className="border-line bg-bg-soft rounded-[var(--radius-panel)] border p-4"
      data-testid="compose"
    >
      <div className="flex items-center gap-2">
        <Mail className="text-ai size-4 shrink-0" aria-hidden="true" />
        <h2 className="flex-1 text-sm font-semibold">{t("compose.title")}</h2>
        <Button variant="ghost" size="sm" onClick={() => setOpen((o) => !o)}>
          {open ? t("common.close") : t("compose.open")}
        </Button>
      </div>

      {open && (
        <>
          <p className="text-fg-faint text-micro mt-1">{t("compose.hint")}</p>

          <div className="mt-3 flex flex-wrap items-end gap-2">
            <SegmentedControl
              size="sm"
              value={kind}
              onChange={setKind}
              options={KINDS.map((value) => ({ value, label: t(`compose.kind_${value}`) }))}
              label={t("compose.kind")}
            />
            <SegmentedControl
              size="sm"
              value={tone}
              onChange={setTone}
              options={TONES.map((value) => ({ value, label: t(`compose.tone_${value}`) }))}
              label={t("compose.tone")}
            />
          </div>

          <div className="mt-2 flex flex-wrap items-end gap-2">
            <Labelled label={t("compose.audience")} className="min-w-[14rem] flex-1">
              <Input
                value={audience}
                onChange={(e) => setAudience(e.target.value)}
                placeholder={t("compose.audience_placeholder")}
              />
            </Labelled>
            <Button onClick={() => void run()} disabled={busy}>
              {busy ? t("compose.working") : draft ? t("compose.again") : t("compose.write")}
            </Button>
          </div>

          {error && (
            <p role="alert" className="text-danger mt-3 text-sm">
              {error}
            </p>
          )}

          {draft && (
            <div className="mt-4 space-y-2">
              {draft.kind === "email" && (
                <Labelled label={t("compose.subject")}>
                  <Input value={subject} onChange={(e) => setSubject(e.target.value)} />
                </Labelled>
              )}
              <Labelled label={t("compose.body")}>
                <TextArea
                  value={body}
                  onChange={(e) => setBody(e.target.value)}
                  rows={10}
                  data-testid="compose-body"
                />
              </Labelled>

              <div className="flex flex-wrap items-center gap-2">
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() =>
                    void copy(wholeMessage).then((ok) => {
                      setCopied(ok);
                      if (!ok) setError(t("compose.copy_failed"));
                    })
                  }
                >
                  {copied ? t("compose.copied") : t("compose.copy")}
                </Button>
                {draft.kind === "email" && (
                  // Built from what is on screen rather than from what the model returned: the
                  // point of showing a draft is that it gets edited, and a link made before the
                  // edit opens the mail client with the old text.
                  <a
                    href={mailto(subject, body)}
                    className="text-accent text-meta rounded-full px-2.5 py-1 font-medium hover:underline"
                  >
                    {t("compose.open_mail")}
                  </a>
                )}
                <Button variant="ghost" size="sm" onClick={() => void keep()}>
                  {t("compose.keep")}
                </Button>
                {note && <span className="text-fg-faint text-micro">{t("compose.kept")}</span>}
              </div>
            </div>
          )}
        </>
      )}
    </section>
  );
}
