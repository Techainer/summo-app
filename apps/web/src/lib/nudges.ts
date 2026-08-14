import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * What the agent wants to say, and when it is allowed to say it.
 *
 * The *decision* lives in the daemon (`summo-engine/src/nudge.rs`), not here. That is deliberate:
 * the same rules then apply to a desktop notification, an in-app banner and a mobile push, instead
 * of each surface inventing its own idea of "too often".
 *
 * Asking for one is also the act of consuming it — the daemon records what it said, so a nudge is
 * delivered once even if two windows are open.
 */

export type Reason =
  "daily-report" | "weekly-rollup" | "draft-waiting" | "overdue" | "meeting-soon";

export interface Nudge {
  reason: Reason;
  title: string;
  body: string;
  /** Where tapping it should go. */
  route: string;
  key: string;
}

export class NudgeClient {
  constructor(private readonly handshake: Handshake) {}

  async due(): Promise<Nudge[]> {
    return readJson<Nudge[]>(await fetch(url(this.handshake, "/nudges")));
  }
}

/**
 * How often to ask.
 *
 * Five minutes. It was fifteen, which was a fine compromise while every nudge was about a day that
 * had already happened — but "your meeting starts in five minutes" is only worth saying inside a
 * window about that wide, and a fifteen-minute timer lands inside it barely more often than not.
 * A request to a daemon on the same machine is not what wakes a laptop.
 */
export const POLL_MS = 5 * 60 * 1000;

/**
 * Show a nudge as an OS notification, if the user has allowed it.
 *
 * Never asks for permission on its own. A permission prompt on first launch, before the app has
 * done anything useful, is how an app teaches people to click "Block" — so this only fires when
 * permission is already granted, and asking is a deliberate action in Settings.
 */
export function notify(nudge: Nudge, onClick: (route: string) => void): boolean {
  if (typeof Notification === "undefined" || Notification.permission !== "granted") return false;

  const notification = new Notification(nudge.title, {
    body: nudge.body,
    // One notification per thing: a second "3 việc còn treo" should replace the first, not stack.
    tag: nudge.key,
  });
  notification.onclick = () => {
    window.focus();
    onClick(nudge.route);
    notification.close();
  };
  return true;
}

/** Ask for permission. Only from a deliberate action, never on load. */
export async function requestPermission(): Promise<boolean> {
  if (typeof Notification === "undefined") return false;
  if (Notification.permission === "granted") return true;
  if (Notification.permission === "denied") return false;
  return (await Notification.requestPermission()) === "granted";
}

/** Whether the browser will show notifications right now. */
export function canNotify(): boolean {
  return typeof Notification !== "undefined" && Notification.permission === "granted";
}

/** An icon for a nudge, so the banner is scannable without reading it. */
export function iconFor(reason: Reason): string {
  switch (reason) {
    case "draft-waiting":
      return "✎";
    case "overdue":
      return "!";
    case "weekly-rollup":
      return "◷";
    case "meeting-soon":
      return "◎";
    default:
      return "◔";
  }
}
