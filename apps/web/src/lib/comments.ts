import type { Handshake } from "./engine";
import { url } from "./library";

/**
 * What people said *about* a meeting, as opposed to in it.
 *
 * The same thread carries the agent's proposals, which is the point: a comment and "shall I add
 * this as a task?" are the same conversation, and splitting them into two panels would make the
 * agent something you check on rather than something you talk to.
 */

export type Kind = "comment" | "proposal" | "question";
export type Resolution = "open" | "accepted" | "rejected" | "answered";

export type Anchor =
  | { on: "note" }
  | { on: "segment"; seq: number }
  | { on: "section"; heading: string }
  | { on: "task"; id: string };

export interface Reaction {
  emoji: string;
  by: string[];
}

export interface Annotation {
  id: string;
  kind: Kind;
  author: string;
  /** ISO-8601 with the offset it was written in. */
  at: string;
  body: string;
  anchor: Anchor;
  resolution: Resolution;
  reactions?: Reaction[];
}

export interface Thread {
  annotations: Annotation[];
  /** Proposals nobody has answered. Counted by the daemon so two clients cannot disagree. */
  pending: number;
}

async function json<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(body?.error ?? `${response.status} ${response.statusText}`);
  }
  return (await response.json()) as T;
}

export interface Where {
  seq?: number;
  heading?: string;
}

export class CommentClient {
  constructor(
    private readonly handshake: Handshake,
    private readonly meeting: string,
  ) {}

  private at(suffix = ""): string {
    return url(this.handshake, `/meetings/${encodeURIComponent(this.meeting)}/comments${suffix}`);
  }

  async list(): Promise<Thread> {
    return json<Thread>(await fetch(this.at()));
  }

  async add(body: string, author: string, where: Where = {}): Promise<Annotation> {
    return json<Annotation>(
      await fetch(this.at(), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ body, author, ...where }),
      }),
    );
  }

  async react(id: string, emoji: string, by: string): Promise<void> {
    await json(
      await fetch(this.at(`/${encodeURIComponent(id)}/react`), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ emoji, by }),
      }),
    );
  }

  async remove(id: string): Promise<boolean> {
    const body = await json<{ removed: boolean }>(
      await fetch(this.at(`/${encodeURIComponent(id)}`), { method: "DELETE" }),
    );
    return body.removed;
  }
}

/** The reactions worth offering without a picker. */
export const QUICK = ["👍", "🎉", "❓", "👀"] as const;

/**
 * A short label for where a comment is pinned.
 *
 * Returns `null` for a note-level comment, so the common case renders no chip at all — a badge
 * saying "this is about the note" on every comment in a note is noise.
 */
export function anchorLabel(anchor: Anchor): string | null {
  switch (anchor.on) {
    case "segment":
      return `#${anchor.seq}`;
    case "section":
      return anchor.heading;
    case "task":
      return anchor.id;
    default:
      return null;
  }
}

/** Whether a comment is pinned to one utterance, and to which. */
export function segmentOf(anchor: Anchor): number | null {
  return anchor.on === "segment" ? anchor.seq : null;
}

/**
 * Comments in the order they were said.
 *
 * Sorted by the written timestamp rather than by arrival: the file is the source of truth and a
 * user may have edited it by hand, in which case the order in the array means nothing.
 */
export function inOrder(annotations: Annotation[]): Annotation[] {
  return [...annotations].sort((a, b) => a.at.localeCompare(b.at));
}

/**
 * `HH:MM` from an ISO timestamp, keeping the offset it was written in.
 *
 * Deliberately string-sliced rather than parsed through `Date`: a comment written at 18:00 in Hanoi
 * should read 18:00 to whoever wrote it, and `Date` would re-render it in the reader's zone —
 * turning their own words into a different hour than they typed them.
 */
export function writtenAt(iso: string): string {
  const match = /T(\d{2}:\d{2})/.exec(iso);
  return match?.[1] ?? "";
}

/** Whether the current user has already reacted with this emoji. */
export function reacted(annotation: Annotation, emoji: string, me: string): boolean {
  return annotation.reactions?.some((r) => r.emoji === emoji && r.by.includes(me)) ?? false;
}
