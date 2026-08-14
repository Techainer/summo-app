import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Drafting the thing you have to send after the meeting.
 *
 * The app has, until now, ended at a good note — which is where the actual work starts for most
 * people: somebody still has to tell the three colleagues who were not there. This turns what the
 * meeting produced into a message, in one of four shapes, and then gets out of the way.
 *
 * **Nothing is sent from here.** The draft is text on screen: copy it, or open your own mail client
 * through a `mailto:` link with the subject and body already filled in. Summo holds no mail
 * account, so it cannot send the wrong draft to a customer — and the last edit is always yours.
 */

export type Kind = "email" | "message" | "recap" | "actions";
export type Tone = "neutral" | "friendly" | "formal";

export interface Composed {
  kind: Kind;
  /** An email has one; a chat message does not, and inventing one would eat its first line. */
  subject: string | null;
  body: string;
  /** Only for an email. */
  mailto: string | null;
}

export interface Ask {
  kind: Kind;
  tone: Tone;
  /** Who reads it, in the user's own words. Optional, and it changes the writing more than tone. */
  audience?: string;
}

export class ComposeClient {
  constructor(private readonly handshake: Handshake) {}

  async compose(meeting: string, ask: Ask): Promise<Composed> {
    return readJson<Composed>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/compose`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(ask),
      }),
    );
  }

  /** Keep it as a note, so it outlives the tab. */
  async save(meeting: string, title: string, body: string): Promise<string> {
    const result = await readJson<{ note: string }>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/compose/save`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ title, body }),
      }),
    );
    return result.note;
  }
}

/**
 * A `mailto:` for text the user has since edited.
 *
 * The daemon builds one from what the model wrote, and it is stale the moment anybody fixes a
 * sentence — which they will, since the whole point of showing the draft is that it is a draft.
 * Built here from the current contents, so the mail client opens with what is on screen.
 */
export function mailto(subject: string | null, body: string): string {
  const parts = [];
  if (subject?.trim()) parts.push(`subject=${encodeURIComponent(subject)}`);
  parts.push(`body=${encodeURIComponent(body)}`);
  return `mailto:?${parts.join("&")}`;
}

/**
 * Put text on the clipboard, and say whether it worked.
 *
 * `navigator.clipboard` is unavailable over plain HTTP on some browsers and refused without a user
 * gesture on others, and a copy button that silently does nothing is worse than one that admits it
 * — the user walks away believing they have the text.
 */
export async function copy(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}
