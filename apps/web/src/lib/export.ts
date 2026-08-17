import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Getting a meeting back out, and getting it in another language.
 *
 * Both halves have been in the daemon since before the interface had a page screen, and neither had
 * anywhere to be reached from. `summo_vault::export` writes six formats and the route serving them
 * was called by nothing; `/meetings/{id}/translate` translates a recording that was captured
 * without live translation, and the transcript has always *rendered* a translation it happened to
 * find — the reading half built, the writing half unreachable.
 *
 * Exporting is the more ordinary of the two and the more often wanted: what people do after a
 * meeting is send somebody the part they missed.
 */

/** What the daemon can write. `md` is the vault's own file, unchanged. */
export const FORMATS = ["txt", "md", "srt", "vtt", "csv", "json"] as const;
export type Format = (typeof FORMATS)[number];

/** How far a translation got. `complete` is false when the model skipped lines. */
export interface Translated {
  lang: string;
  translated: number;
  missing: number;
  requests: number;
  complete: boolean;
}

export class ExportClient {
  constructor(private readonly handshake: Handshake) {}

  /** Which languages this meeting already exists in, besides the one it was spoken in. */
  async languages(meeting: string): Promise<string[]> {
    return readJson<string[]>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/translations`)),
    );
  }

  /**
   * The exported text.
   *
   * Fetched rather than linked so a failure is an error message instead of a browser window
   * holding `{"error": …}` — the daemon answers this route with a subtitle file on success and
   * JSON on failure, and a download attribute cannot tell the difference.
   */
  async text(meeting: string, format: Format, lang?: string): Promise<string> {
    const response = await fetch(
      url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/subtitles`, {
        format,
        ...(lang ? { lang } : {}),
      }),
    );
    const body = await response.text();
    if (!response.ok) {
      let said: unknown = body;
      try {
        said = (JSON.parse(body) as { error?: unknown }).error ?? body;
      } catch {
        // Not JSON, so the body is the message.
      }
      throw new Error(typeof said === "string" ? said : response.statusText);
    }
    return body;
  }

  /** Translate the whole meeting, writing a file beside it in the vault. */
  async translate(meeting: string, lang: string): Promise<Translated> {
    return readJson<Translated>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/translate`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ lang }),
      }),
    );
  }
}

/**
 * Hand a string to the browser as a file.
 *
 * An object URL rather than a `data:` one: a transcript is easily a megabyte, and a data URL that
 * long is refused outright by some webviews — including the one this app ships inside.
 */
export function download(name: string, text: string): void {
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const href = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = href;
  link.download = name;
  document.body.append(link);
  link.click();
  link.remove();
  // Released on the next turn of the loop rather than immediately: revoking inside the same tick
  // races the download the click just started, and the file arrives empty.
  setTimeout(() => URL.revokeObjectURL(href), 0);
}

/** `2026-08-16-hop-dau-tuan.srt` — the file name a person would have typed. */
export function fileName(title: string, day: string, format: Format, lang?: string): string {
  const slug =
    title
      .normalize("NFD")
      .replace(/[\u0300-\u036f]/g, "")
      // `\u0110`/`\u0111` are Đ and đ: a letter of its own rather than D with a mark on it, so the
      // decomposition above leaves it untouched and every slug would keep a character no Windows
      // share accepts.
      .replace(/[\u0110\u0111]/g, "d")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "summo";
  return [day, slug, lang].filter(Boolean).join("-") + `.${format}`;
}
