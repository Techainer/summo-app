/**
 * Naming the voices in a meeting.
 *
 * Diarization produces `S2`, `S3` — labels, not people. This is the edge that turns them into
 * names, and the important part is what happens afterwards: naming a voice does not only fix the
 * meeting on screen, it re-sweeps every past meeting where the same voice was misattributed. The
 * daemon does that work; the interface's job is to say so, because a correction that silently
 * rewrites eleven old transcripts is alarming when it is not explained.
 */

import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

export interface Person {
  id: string;
  name: string;
  avatar?: string;
  /** Utterances that shaped this profile. */
  samples: number;
  /** Of those, how many a human confirmed rather than the model guessed. */
  confirmed: number;
  /** Distinct ways this voice has sounded — headset, laptop mic, phone. */
  centroids: number;
}

export interface PeopleView {
  people: Person[];
  /** The embedding model these profiles belong to, e.g. `campplus-sv/192d`. */
  space?: string;
}

export interface Suggestion {
  id: string;
  name: string;
  /** Cosine similarity, 0 to 1. */
  similarity: number;
}

export interface UnknownVoice {
  /** The provisional label, e.g. `S2`. */
  label: string;
  utterances: number;
  seconds: number;
  suggestions: Suggestion[];
}

/**
 * One meeting's unnamed voices.
 *
 * The voice book asks about the whole vault rather than about one recording. It had to: the
 * per-meeting question was only ever rendered by a component nothing passed a meeting to, so the
 * naming interface — the entire point of the screen — has never been reachable.
 */
export interface MeetingUnknowns {
  meeting: string;
  title: string;
  day: string;
  voices: UnknownVoice[];
}

export interface MeetingChange {
  meeting: string;
  utterances: number;
}

export interface Correction {
  person: Person;
  relabelled_here: number;
  relabelled_elsewhere: MeetingChange[];
  corrected_profiles: string[];
}

export class PeopleClient {
  constructor(private readonly handshake: Handshake) {}

  private post<T>(path: string, body?: unknown): Promise<T> {
    return fetch(url(this.handshake, path), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    }).then(readJson<T>);
  }

  async list(): Promise<PeopleView> {
    return readJson<PeopleView>(await fetch(url(this.handshake, "/people")));
  }

  async unknowns(meeting: string): Promise<UnknownVoice[]> {
    return readJson<UnknownVoice[]>(
      await fetch(url(this.handshake, `/meetings/${encodeURIComponent(meeting)}/voices`)),
    );
  }

  /** Everything still unnamed, anywhere in the vault, newest meeting first. */
  async unnamed(): Promise<MeetingUnknowns[]> {
    return readJson<MeetingUnknowns[]>(await fetch(url(this.handshake, "/voices/unknown")));
  }

  /** Name a voice. Fixes this meeting and every past one that guessed it wrong. */
  nameVoice(meeting: string, label: string, name: string): Promise<Correction> {
    return this.post<Correction>(
      `/meetings/${encodeURIComponent(meeting)}/voices/${encodeURIComponent(label)}`,
      { name },
    );
  }

  rename(id: string, name: string): Promise<Person> {
    return this.post<Person>(`/people/${encodeURIComponent(id)}/name`, {
      name,
    });
  }

  setAvatar(id: string, avatar: string | null): Promise<Person> {
    return this.post<Person>(`/people/${encodeURIComponent(id)}/avatar`, {
      avatar,
    });
  }

  /** Fold `from` into `into`. `from` disappears. */
  merge(into: string, from: string): Promise<Person> {
    return this.post<Person>(`/people/${encodeURIComponent(into)}/merge`, {
      from,
    });
  }

  async forget(id: string): Promise<boolean> {
    const body = await readJson<{ removed: boolean }>(
      await fetch(url(this.handshake, `/people/${encodeURIComponent(id)}`), {
        method: "DELETE",
      }),
    );
    return body.removed;
  }
}

/**
 * How sure the match is, in words rather than a number — as a translation key.
 *
 * A percentage invites the user to reason about a cosine similarity, which is not a thing anybody
 * should have to do to name a colleague. The bands are wide on purpose.
 *
 * A key rather than the words, because this module is pure logic and has no locale: choosing the
 * band is arithmetic, and saying it is the caller's job. It used to return Vietnamese, so the
 * English interface offered "Bình (rất giống)".
 */
export function confidenceLabel(similarity: number): string {
  if (similarity >= 0.75) return "people.similarity_high";
  if (similarity >= 0.62) return "people.similarity_mid";
  return "people.similarity_low";
}

/**
 * What a correction did, phrased for a person.
 *
 * Returns nothing when nothing beyond the obvious happened — there is no point announcing that naming
 * a voice named the voice.
 */
export function correctionSummary(correction: Correction): Phrase[] {
  const parts: Phrase[] = [];

  const elsewhere = correction.relabelled_elsewhere;
  if (elsewhere.length > 0) {
    parts.push({
      key: "people.relabelled",
      params: {
        utterances: String(elsewhere.reduce((total, m) => total + m.utterances, 0)),
        meetings: String(elsewhere.length),
      },
    });
  }

  if (correction.corrected_profiles.length > 0) {
    parts.push({
      key: "people.unmerged",
      params: { count: String(correction.corrected_profiles.length) },
    });
  }

  return parts;
}

/** Something to say, named rather than spelled — see `correctionSummary`. */
export interface Phrase {
  key: string;
  params: Record<string, string>;
}

/**
 * People worth offering when naming a voice, best guess first.
 *
 * The suggestions the daemon scored come first in its order, then everyone else alphabetically —
 * because the right answer is often somebody the model did not recognise at all, and that person
 * still has to be reachable without typing their name again.
 */
export function nameOptions(voice: UnknownVoice, people: Person[]): Person[] {
  const suggested = voice.suggestions
    .map((s) => people.find((p) => p.id === s.id))
    .filter((p): p is Person => p !== undefined);
  const suggestedIds = new Set(suggested.map((p) => p.id));
  const rest = people
    .filter((p) => !suggestedIds.has(p.id))
    .sort((a, b) => a.name.localeCompare(b.name, "vi"));
  return [...suggested, ...rest];
}
