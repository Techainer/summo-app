import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * Asking an agent for something, and what this person keeps asking for.
 *
 * Both halves of the same idea. The ask goes to `/agent/run`, which turns a sentence into a task
 * the agent owns and leaves its work in the vault as Markdown. The daemon writes the sentence down
 * as it goes, and anything typed more than once comes back as a habit — so the interface can offer
 * the words instead of asking for them again, and the agent can be told what standard the earlier
 * answers set.
 *
 * The habits file is `vault/agents/HABITS.md`. It is readable, and deleting a line forgets it.
 */

export interface Habit {
  /** The most recent phrasing, which is the one worth offering back. */
  instruction: string;
  times: number;
  /** ISO date of the last time it was asked. */
  last: string;
}

export async function fetchHabits(handshake: Handshake): Promise<Habit[]> {
  return readJson<Habit[]>(await fetch(url(handshake, "/agent/habits")));
}

/**
 * Hand an agent a sentence about a note.
 *
 * `meeting` is what it is being asked *about*, and it travels with the instruction so the agent
 * has the context and the habit knows what kind of note it tends to be asked about.
 */
export async function askAgent(
  handshake: Handshake,
  instruction: string,
  meeting?: string,
): Promise<{ task: string }> {
  return readJson<{ task: string }>(
    await fetch(url(handshake, "/agent/run"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ instruction, meeting: meeting ?? null }),
    }),
  );
}
