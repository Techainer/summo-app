import { useCallback, useState } from "react";

import { useT } from "../../i18n/context";
import {
  detectBrowser,
  detectPlatform,
  inputDevices,
  micRecovery,
  micState,
  requestMic,
  systemAudio,
  type MicState,
  type Platform,
} from "../../lib/permissions";
import { useLoad } from "../../lib/use-load";
import { Button } from "../ui";

/**
 * The microphone permission, asked for and repaired in the app.
 *
 * This exists because the failure it prevents is silent and total. A user presses record, the
 * operating system refuses, and the app has one sentence to explain a checkbox in a settings pane
 * they have never opened — on macOS the system prompt appears *once ever*, so somebody who
 * dismissed it while reading something else has no way back except through this panel.
 *
 * Three rules:
 *
 * **The request is a button.** Browsers only prompt from a user gesture, and asking on load — before
 * the app has shown what it is for — is how a permission gets refused permanently by somebody who
 * was only looking around. Nothing here runs until it is clicked, except reading the current state.
 *
 * **The device is released immediately.** The point is the permission; holding the stream would
 * light the recording indicator on a machine that is not recording, which is the one thing a
 * local-first recorder must never do.
 *
 * **The instructions name the place.** Not "check your settings" — the browser's menu, then the
 * operating system's pane, with the application macOS will actually list, which is the browser and
 * not Summo.
 */
export function Permissions({ compact = false }: { compact?: boolean }) {
  const t = useT();
  const [asked, setAsked] = useState<{ state: MicState; devices: MediaDeviceInfo[] } | null>(null);
  const [asking, setAsking] = useState(false);
  // Detected during the first render rather than in an effect: the user agent is available before
  // paint and does not change, so an effect would only add a second render that flashes the wrong
  // operating system's instructions. `chosen` overrides it — a wrong guess costs the reader steps
  // for a machine they are not using, and being able to say so is cheaper than always being right.
  const [chosen, setChosen] = useState<Platform | null>(null);
  const platform = chosen ?? detectPlatform();
  const setPlatform = setChosen;
  const browser = detectBrowser();

  // Read on mount and on demand, through the shared loader: a `setState` in an effect body is what
  // `react-hooks/set-state-in-effect` exists to catch, and this is the one place in the app allowed
  // to answer it — see `use-load.ts`.
  const probe = useLoad(
    useCallback(async () => ({ state: await micState(), devices: await inputDevices() }), []),
    [],
  );
  const read = probe.reload;

  // What the panel shows: the answer to an explicit request wins over the last probe, because after
  // clicking "allow" the browser's `permissions.query` may still say `prompt` — Chromium does not
  // update it for a stream that was granted and released.
  const state: MicState = asked?.state ?? probe.data?.state ?? "unknown";
  const devices = asked?.devices ?? probe.data?.devices ?? [];

  const ask = async () => {
    setAsking(true);
    try {
      setAsked({ state: await requestMic(), devices: await inputDevices() });
    } finally {
      setAsking(false);
    }
  };

  const system = systemAudio(platform);
  const named = devices.filter((device) => device.label.trim().length > 0);

  return (
    <section className="border-line bg-bg-raised rounded-2xl border p-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h2 className="font-medium">{t("permissions.title")}</h2>
          <p className="text-fg-dim text-meta mt-1">{t("permissions.why")}</p>
        </div>
        <MicBadge state={state} />
      </div>

      {/* Granted: say what was found rather than only that it worked. A user who sees the name of
          the wrong microphone here learns something the app cannot tell them any other way. */}
      {state === "granted" && (
        <p className="text-fg-dim text-meta mt-3">
          {named[0]
            ? t("permissions.found", { count: String(devices.length), name: named[0].label })
            : t("permissions.found_unnamed", { count: String(devices.length) })}
        </p>
      )}

      {state !== "granted" && (
        <div className="mt-4 flex flex-wrap items-center gap-2">
          <Button onClick={() => void ask()} disabled={asking}>
            {asking ? t("permissions.asking") : t("permissions.ask")}
          </Button>
          <Button
            variant="ghost"
            onClick={() => {
              setAsked(null);
              read();
            }}
          >
            {t("permissions.recheck")}
          </Button>
        </div>
      )}

      {/* Steps only once refused. Showing the repair path to somebody who has not been asked yet
          makes a first run look like a problem. */}
      {state === "denied" && (
        <ol className="text-fg-dim text-meta mt-4 list-decimal space-y-2 pl-5">
          {micRecovery(platform, browser).map((step) => (
            <li key={step.key}>{t(step.key, step.values)}</li>
          ))}
        </ol>
      )}

      {state === "denied" && (
        <p className="text-fg-faint text-micro mt-3">
          {t("permissions.wrong_os")}{" "}
          {(["macos", "windows", "linux"] as const)
            .filter((other) => other !== platform)
            .map((other) => (
              <button
                key={other}
                type="button"
                onClick={() => setPlatform(other)}
                className="mr-2 underline"
              >
                {t(`permissions.os_${other}`)}
              </button>
            ))}
        </p>
      )}

      {!compact && (
        <div className="border-line mt-5 border-t pt-4">
          <h3 className="text-meta font-medium">{t("permissions.system_title")}</h3>
          <p className="text-fg-dim text-meta mt-1">{t(system.key)}</p>
        </div>
      )}
    </section>
  );
}

/** The current answer, in one word and one colour. */
function MicBadge({ state }: { state: MicState }) {
  const t = useT();
  const styles: Record<MicState, string> = {
    granted: "bg-accent-soft text-done border-accent/30",
    denied: "bg-rec-soft text-rec border-rec/30",
    prompt: "bg-bg-soft text-fg-faint border-line",
    unknown: "bg-bg-soft text-fg-faint border-line",
  };
  return (
    <span className={`text-micro rounded-full border px-2 py-1 ${styles[state]}`}>
      {t(`permissions.state_${state}`)}
    </span>
  );
}
