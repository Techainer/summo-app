import { useCallback } from "react";

import type { Translator } from "../i18n";
import { useI18n } from "../i18n/context";

/**
 * Errors from the daemon, in the language the user is reading.
 *
 * The daemon writes one language. The interface is translated into whatever the user chose. Without
 * something in between, every failure arrives as Vietnamese text on an English screen — the app
 * speaking two languages at once, and always the wrong one at the worst moment.
 *
 * So the daemon sends a stable `code` alongside its `error` text, and this picks the translation
 * when there is one. **The text is the fallback, always.** A code the interface has never seen still
 * has to say something useful, which means adding a code on the Rust side is never a breaking
 * change and a client that is a version behind keeps working.
 *
 * Not everything gets a code, on purpose. A checksum mismatch or an unreadable file is a *fault*,
 * not a message: the detail is the whole content, and a translated "something went wrong" would be
 * strictly worse than the path and the reason.
 */

export interface Failure {
  /** Human text the daemon wrote. Always present. */
  error: string;
  /** Stable key, when the daemon had one for this. */
  code?: string;
}

/** Pull a failure out of a response body, whatever shape it turned out to be. */
export function failureFrom(body: unknown, status?: number): Failure {
  if (typeof body === "object" && body !== null) {
    const record = body as { error?: unknown; code?: unknown };
    if (typeof record.error === "string" && record.error.length > 0) {
      return {
        error: record.error,
        ...(typeof record.code === "string" && record.code.length > 0 ? { code: record.code } : {}),
      };
    }
  }
  // A body that is not an error object at all — an HTML error page from a proxy, an empty 502.
  // Saying the status beats saying "[object Object]".
  return { error: status ? `HTTP ${status}` : "unknown error" };
}

/**
 * The sentence to show.
 *
 * `t.has` rather than a try/catch: `t` renders a missing key as the key itself, which on screen
 * would read `errors.import.no_audio` — worse than the Vietnamese it replaced.
 */
export function explain(failure: Failure, t: Translator): string {
  if (failure.code) {
    const key = `errors.${failure.code}`;
    if (t.has(key)) return t.t(key);
  }
  return failure.error;
}

/**
 * Turn anything thrown into something showable.
 *
 * Clients in this app throw `Error` with the daemon's message already in it, so the common path is
 * one line. The rest is for the genuinely unexpected — a string thrown by a library, a rejected
 * promise with no reason — which should still not render as "undefined".
 */
export function messageOf(thrown: unknown): string {
  if (thrown instanceof Error) return thrown.message;
  if (typeof thrown === "string" && thrown.length > 0) return thrown;
  return "unknown error";
}

/**
 * An error carrying the daemon's code as well as its text.
 *
 * A plain `Error` loses the code the moment it is thrown, which is how ten separate copies of the
 * same fetch helper ended up discarding it. Catch sites that do not care still just read
 * `.message`.
 */
export class DaemonError extends Error {
  readonly code?: string;

  constructor(failure: Failure) {
    super(failure.error);
    this.name = "DaemonError";
    if (failure.code) this.code = failure.code;
  }
}

/**
 * Read a JSON response, or throw something that can be shown to a user.
 *
 * One copy, shared by every client. There used to be ten near-identical ones, and every one of them
 * threw away the `code` — so a fix in any of them fixed a tenth of the problem.
 */
export async function readJson<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const body: unknown = await response.json().catch(() => null);
    throw new DaemonError(failureFrom(body, response.status));
  }
  return (await response.json()) as T;
}

/**
 * What to show for something that was thrown, translated when it can be.
 *
 * The one call sites want: `catch (e) { setError(describeError(e, t)) }`.
 */
export function describeError(thrown: unknown, t: Translator): string {
  if (thrown instanceof DaemonError) {
    return explain({ error: thrown.message, ...(thrown.code ? { code: thrown.code } : {}) }, t);
  }
  return messageOf(thrown);
}

/**
 * The hook a component wants: one function that turns anything thrown into a sentence.
 *
 * `catch (e) { setError(say(e)) }` at every call site, rather than each one deciding for itself
 * whether to translate — which is how the daemon's Vietnamese ended up on English screens in the
 * first place.
 */
export function useErrorText(): (thrown: unknown) => string {
  const i18n = useI18n();
  return useCallback((thrown: unknown) => describeError(thrown, i18n), [i18n]);
}
