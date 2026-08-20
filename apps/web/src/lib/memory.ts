import { useEffect, useState } from "react";

import { url } from "./library";
import type { Handshake } from "./engine";

/** What the status bar shows: how much of this machine's memory is in use. */
export interface Memory {
  usedGb: number;
  totalGb: number;
}

/** Re-read every fifteen seconds. Often enough to be live, rare enough to be free. */
const EVERY_MS = 15_000;

/**
 * The machine's memory, in the corner of the window.
 *
 * Added because a user asked for it, after three releases spent chasing a bug whose cause was a
 * memory reading: their Mac reported zero bytes free on a 24 GB laptop, every model was judged too
 * large to run, and the number that decided it was never on screen. A figure the app acts on should
 * be a figure the person can see.
 *
 * `/hw` rather than `/health`: the machine's shape is behind the token, and `/health` is the
 * one route without one — a daemon on loopback should not describe the computer to any page that
 * happens to guess the port.
 */
export function useMemory(handshake: Handshake | null): Memory | null {
  const [memory, setMemory] = useState<Memory | null>(null);

  useEffect(() => {
    if (!handshake) return undefined;
    let stopped = false;

    const read = async () => {
      try {
        const response = await fetch(url(handshake, "/hw"), { cache: "no-store" });
        if (!response.ok) return;
        const body = (await response.json()) as {
          total_ram_mb?: number;
          available_ram_mb?: number;
        };
        const total = body.total_ram_mb ?? 0;
        const available = body.available_ram_mb ?? 0;
        if (stopped || total <= 0) return;
        setMemory({
          usedGb: Math.max(0, (total - available) / 1024),
          totalGb: total / 1024,
        });
      } catch {
        // The status bar already says when the daemon is unreachable; a second way of saying it
        // would be noise, and a stale figure is better than a flickering one.
      }
    };

    void read();
    const timer = window.setInterval(() => void read(), EVERY_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [handshake]);

  return memory;
}
