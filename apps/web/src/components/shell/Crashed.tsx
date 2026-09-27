import { RotateCw } from "lucide-react";

import { useT } from "../../i18n/context";
import { Button } from "../ui";

/**
 * What a person sees when a screen throws.
 *
 * The router ships a default for this and it is one line of English — **"Something went wrong!"**
 * beside a button reading "Show Error" — rendered over the whole app. Nothing named it, so nothing
 * translated it, and a Vietnamese user meeting the worst moment in the product met it in a language
 * the rest of the app had been careful never to use.
 *
 * It also said nothing true about the one thing they would be worried about. A crash in the
 * interface does not touch the recording: the daemon is a separate process, it holds the meeting,
 * and it goes on writing the file while this is on screen. That sentence is the whole reason this
 * component exists — the difference between "I lost the meeting" and "I press reload".
 *
 * The error itself stays, behind a disclosure, because the person who can use it is not the person
 * who needs reassuring and they are occasionally the same person.
 */
export function Crashed({ error }: { error: unknown }) {
  const t = useT();
  const detail = error instanceof Error ? (error.stack ?? error.message) : String(error);

  return (
    <div
      role="alert"
      className="mx-auto flex max-w-xl flex-col items-start gap-3 px-6 py-16"
      data-testid="crashed"
    >
      <h1 className="text-title font-semibold">{t("common.crash_title")}</h1>
      <p className="text-fg-dim text-body">{t("common.crash_body")}</p>

      <Button onClick={() => window.location.reload()} className="mt-1">
        <RotateCw aria-hidden="true" className="me-1.5 size-4" />
        {t("common.crash_reload")}
      </Button>

      <details className="mt-3 w-full">
        <summary className="text-fg-faint text-meta cursor-pointer">
          {t("common.crash_details")}
        </summary>
        {/* Selectable and wrapped: the useful thing to do with a stack trace is send it to
            somebody, and one that needs horizontal scrolling to read cannot be copied in one go. */}
        <pre className="text-fg-dim text-micro border-line bg-bg-soft rounded-card mt-2 max-h-64 overflow-auto border p-3 whitespace-pre-wrap">
          {detail}
        </pre>
      </details>
    </div>
  );
}
