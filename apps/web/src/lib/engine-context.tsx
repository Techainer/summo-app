import { createContext, useContext } from "react";

import type { Failure } from "./errors";
import type { LibraryClient } from "./library";
import type { PeopleClient } from "./people";
import type { SessionState } from "./session";
import type { TranscriptState } from "./transcript";

/** Where the daemon is, when the app was not launched by the shell. */
export const DEV_HANDSHAKE = { port: 8710, token: "" };

export const IDLE: SessionState = {
  recording: false,
  connection: "closed",
  error: null,
  deviceLabel: null,
  sampleRate: null,
  meeting: null,
};

export interface Stat {
  rtf: number;
  rss_mb: number;
  queue_ms: number;
}

export interface EngineValue {
  library: LibraryClient;
  people: PeopleClient;
  handshake: { port: number; token: string };
  session: SessionState;
  transcript: TranscriptState;
  elapsed: number;
  level: number;
  stat: Stat | null;
  /** What the socket last said, unsaid: the screen translates it. See `EngineProvider`. */
  notice: Failure | null;
  dismissNotice: () => void;
  start: () => Promise<void>;
  stop: () => void;
  toggle: () => void;
  /**
   * Change the language or the model mid-meeting, without ending it.
   *
   * Either alone, or both together. An omitted field is left as it is, which is what makes "switch
   * to Whisper and keep decoding Vietnamese" expressible — the pair a language-only signature could
   * not say, and the reason the in-meeting banner could offer a language and nothing else.
   */
  retune: (change: { language?: string; model?: string }) => void;
  /** Change the live translation target mid-meeting; the empty string turns it off. */
  translate: (to: string) => void;
  /**
   * Save what the user has typed into the meeting that is running.
   *
   * On the context rather than reached through the session object, because the editor that calls
   * it is the same editor a finished note uses — it should not have to know whether the document
   * it is in happens to be recording.
   */
  notes: (text: string) => void;
}

export const EngineContext = createContext<EngineValue | null>(null);

/**
 * The live connection to the daemon, held above the router.
 *
 * Recording has to survive navigation: opening the library mid-meeting must not tear down the
 * WebSocket. Keeping the session here rather than inside a screen is what makes that true — a
 * screen can unmount and the recording continues.
 */

export function useEngine(): EngineValue {
  const value = useContext(EngineContext);
  if (!value) throw new Error("useEngine must be used inside an EngineProvider");
  return value;
}
