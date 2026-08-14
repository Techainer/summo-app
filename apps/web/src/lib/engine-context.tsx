import { createContext, useContext } from "react";

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
  notice: string | null;
  dismissNotice: () => void;
  start: () => Promise<void>;
  stop: () => void;
  toggle: () => void;
  /** Change the language mid-meeting, without ending it. */
  retune: (language: string) => void;
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
