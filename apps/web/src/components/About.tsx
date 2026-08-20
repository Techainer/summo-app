import { useEffect, useState } from "react";

import { useI18n } from "../i18n/context";
import { useEngine } from "../lib/engine-context";
import { url } from "../lib/library";

/**
 * Who made this, and the one promise worth repeating on every screen that mentions the cloud.
 *
 * Sits at the bottom of Settings rather than behind a menu item nobody opens. Attribution that
 * requires hunting is attribution nobody sees, and the local-only promise belongs next to the
 * language-model settings — the one place in the product where a user's words can leave the
 * machine.
 *
 * Links open in a new tab with `rel="noreferrer noopener"`. `noopener` is the one that matters:
 * without it the opened page gets a handle on this window and can navigate it somewhere else, which
 * is a real attack against an app that holds a session token.
 */
export function About() {
  const { t } = useI18n();
  const { handshake } = useEngine();
  const [version, setVersion] = useState<string | null>(null);

  // The daemon's version, not the interface's. They ship together, but the one that matters in a
  // bug report is the process that did the work — and `/health` is the one route that needs no
  // token, so this shows something even when the credential is wrong.
  useEffect(() => {
    let cancelled = false;
    fetch(url(handshake, "/health"))
      .then((r) => (r.ok ? r.json() : null))
      .then((body: { version?: string } | null) => {
        if (!cancelled && typeof body?.version === "string") setVersion(body.version);
      })
      .catch(() => {
        // No daemon: the rest of the panel is still worth showing.
      });
    return () => {
      cancelled = true;
    };
  }, [handshake]);

  return (
    <section className="border-line mt-10 border-t pt-6 text-sm">
      <h2 className="text-base font-medium">{t("about.title")}</h2>

      {/* The sentence that was here promised audio never leaves the machine. It was deleted when
          summaries grew an endpoint setting and the claim stopped being true; the call to it was
          not, so this section rendered the literal text `about.local_promise`. What Summo does and
          does not send is in Trợ giúp, written accurately and in one place. */}
      <div className="mt-4 flex items-center gap-3">
        <a
          href="https://techainer.com/"
          target="_blank"
          rel="noreferrer noopener"
          className="shrink-0"
        >
          <img
            src="./techainer.png"
            alt="Techainer"
            width={55}
            height={45}
            // Explicit dimensions: the logo loads after the text, and without them the panel jumps
            // the moment it arrives.
            className="h-[45px] w-[55px] object-contain"
          />
        </a>
        <p className="min-w-0">
          <span className="text-fg-faint">{t("about.built_by")} </span>
          <b className="font-medium">Viet Nguyen</b>
          <span className="text-fg-dim"> — {t("about.role")}</span>
          <br />
          <a
            href="https://techainer.com/"
            target="_blank"
            rel="noreferrer noopener"
            className="text-accent hover:underline"
          >
            {t("about.site")}
          </a>
          <span className="text-fg-faint"> · {t("about.company_line")}</span>
        </p>
      </div>

      <p className="text-fg-faint text-meta mt-4">
        {version ? `${t("about.version", { version })} · ` : ""}
        {t("about.source")}
      </p>
    </section>
  );
}
