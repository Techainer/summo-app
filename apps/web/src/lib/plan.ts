import type { Handshake } from "./engine";
import { readJson } from "./errors";
import { url } from "./library";

/**
 * What will actually run when you press record, and whether it can do the job.
 *
 * Three different things decide three different jobs and no screen ever said which was which:
 * recognition is a model from Summo's own registry on this machine, translation is either a model
 * inside the app or a provider, and summaries and answers are a language model that is usually
 * somebody else's server. "Which model am I using" had three answers in three places.
 *
 * Derived by the daemon on every request rather than stored. A saved language→model table goes
 * stale the moment a model is installed or removed; the registry already knows which installed
 * models claim a language, how accurate each was measured on it, and which keeps up with live audio
 * on this machine.
 */

export interface Better {
  id: string;
  name: string;
  /** Measured, `0` when nobody has measured this language. */
  accuracy: number;
  live_capable: boolean;
  reason: string;
}

export interface Plan {
  /** The spoken language, or `null` for "let the model detect". */
  language: string | null;
  speech: {
    model: string | null;
    name: string | null;
    installed: boolean;
    /** False when the chosen model does not claim the chosen language. */
    covers_language: boolean;
    /** A better model *already installed*, when there is one. */
    better: Better | null;
  };
  /** The voice detector. Without it a recording produces no words at all. */
  detector: { installed: boolean; id: string };
  /** The voice fingerprint, which is what lets a transcript name who spoke. */
  speakers: { installed: boolean; id: string };
  /**
   * The translator, and whether it is actually here.
   *
   * `installed` is only meaningful when `local` is true — an endpoint has nothing to install — and
   * it exists because the panel used to infer readiness from `local && model !== null`, which is
   * equally true of a model on disk and a model that has never been downloaded.
   */
  translation: {
    local: boolean;
    provider: string | null;
    model: string | null;
    installed: boolean;
  };
  language_model: { provider: string; model: string | null };
}

export async function fetchPlan(handshake: Handshake): Promise<Plan> {
  return readJson<Plan>(await fetch(url(handshake, "/settings/plan")));
}
