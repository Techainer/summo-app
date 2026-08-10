import type { Handshake } from "./engine";
import { url } from "./library";

/**
 * The summary the agent wrote, before anybody has agreed to it.
 *
 * It is already in the note — marked, not hidden — so this client reads and revises what is on
 * disk rather than holding a copy. Confirming removes the marks; nothing moves.
 */

export interface DraftSection {
  heading: string;
  body: string;
}

export interface Turn {
  /** `you` or `agent`. */
  role: string;
  text: string;
}

export interface Draft {
  meeting: string;
  template: string;
  sections: DraftSection[];
  turns: Turn[];
  revisions: number;
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export class DraftClient {
  constructor(private readonly handshake: Handshake) {}

  private post<T>(meeting: string, action: string, body?: unknown): Promise<T> {
    return fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/draft/${action}`), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body ?? {}),
    }).then(json<T>);
  }

  async get(meeting: string): Promise<Draft | null> {
    return json<Draft | null>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/draft`)),
    );
  }

  generate(meeting: string, template?: string): Promise<Draft> {
    return this.post<Draft>(meeting, "generate", { template });
  }

  /** Rewrite one passage. Everything outside the selection stays byte-identical. */
  refine(meeting: string, heading: string, selection: string, instruction: string): Promise<Draft> {
    return this.post<Draft>(meeting, "refine", { heading, selection, instruction });
  }

  chat(meeting: string, message: string): Promise<Draft> {
    return this.post<Draft>(meeting, "chat", { message });
  }

  confirm(meeting: string): Promise<{ confirmed: string[] }> {
    return this.post<{ confirmed: string[] }>(meeting, "confirm");
  }

  async discard(meeting: string): Promise<boolean> {
    const body = await json<{ removed: boolean }>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/draft`), {
        method: "DELETE",
      }),
    );
    return body.removed;
  }
}

/**
 * Strip HTML comments before showing a section to a person.
 *
 * The vault uses comments to carry machine-readable state — task ids, statuses, draft markers.
 * They are invisible once Markdown is rendered, but these panels draw the source as plain text, so
 * without this the user reads `<!-- id:T1 status:doing -->` in the middle of their own notes.
 */
export function readable(body: string): string {
  return body
    .replace(/<!--[\s\S]*?-->/g, "")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n");
}

/** Which sections of a meeting are still the agent's, by heading. */
export function draftHeadings(draft: Draft | null): Set<string> {
  return new Set((draft?.sections ?? []).map((s) => s.heading));
}

/**
 * The text the user selected, if the selection is entirely inside one element.
 *
 * A selection that starts in one section and ends in another has no single heading to attribute it
 * to, so refining it would rewrite the wrong thing. Returning null makes the caller offer nothing
 * rather than offer something wrong.
 */
export function selectionWithin(root: HTMLElement | null): string | null {
  if (!root) return null;
  const selection = window.getSelection();
  if (!selection || selection.rangeCount === 0 || selection.isCollapsed) return null;

  const range = selection.getRangeAt(0);
  if (!root.contains(range.commonAncestorContainer)) return null;

  const text = selection.toString().trim();
  return text.length > 0 ? text : null;
}

/**
 * Whether a selection is worth offering to refine.
 *
 * One or two words is almost always an accidental double-click, and asking the model to rewrite
 * "the" wastes a request and the user's attention.
 */
export function isRefinable(selection: string): boolean {
  return selection.trim().split(/\s+/).length >= 3;
}
