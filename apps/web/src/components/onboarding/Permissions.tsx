import { useCallback, useState, type ReactNode } from "react";

import { useT } from "../../i18n/context";
import { cn } from "../../lib/cn";
import {
  detectBrowser,
  detectPlatform,
  inputDevices,
  micRecovery,
  micState,
  notificationRecovery,
  notificationState,
  requestMic,
  requestNotifications,
  systemAudio,
  type MicState,
  type Platform,
  type Step,
} from "../../lib/permissions";
import { useLoad } from "../../lib/use-load";
import { Button } from "../ui";

/**
 * Every permission Summo needs, asked for and repaired in the app.
 *
 * This exists because the failures it prevents are silent and total. A user presses record, the
 * operating system refuses, and the app has one sentence to explain a checkbox in a settings pane
 * they have never opened — on macOS the system prompt appears *once, ever*, so somebody who
 * dismissed it while reading something else has no way back except through this panel.
 *
 * Four rules, and they apply to each permission here:
 *
 * **The request is a button.** Browsers only prompt from a user gesture, and asking on load — before
 * the app has shown what it is for — is how a permission gets refused permanently by somebody who
 * was only looking around. Nothing runs until it is clicked, except reading the current state.
 *
 * **Whatever is taken is given back.** The microphone stream is stopped the instant the answer
 * arrives; holding it would light the recording indicator on a machine that is not recording, which
 * is the one thing a local-first recorder must never do.
 *
 * **The instructions name the place.** Not "check your settings" — the browser's menu, then the
 * operating system's pane, with the application the OS will actually list, which is the browser and
 * not Summo. Then how to confirm the fix worked, without restarting anything.
 *
 * **A granted permission stops talking.** Once it is on, the row is a statement — no steps, no
 * button, nothing to do.
 */
export function Permissions({ compact = false }: { compact?: boolean }) {
  const t = useT();
  // Detected during the first render rather than in an effect: the user agent is available before
  // paint and does not change, so an effect would only add a second render that flashes the wrong
  // operating system's instructions. `chosen` overrides it — a wrong guess costs the reader steps
  // for a machine they are not using, and being able to say so is cheaper than always being right.
  const [chosen, setChosen] = useState<Platform | null>(null);
  const platform = chosen ?? detectPlatform();
  const browser = detectBrowser();

  // Read on mount and on demand, through the shared loader: a `setState` in an effect body is what
  // `react-hooks/set-state-in-effect` exists to catch, and `use-load.ts` is the one place in the
  // app allowed to answer it.
  const probe = useLoad(
    useCallback(
      async () => ({
        mic: await micState(),
        devices: await inputDevices(),
        notify: notificationState(),
      }),
      [],
    ),
    [],
  );

  // The answer to an explicit request wins over the last probe: after granting a stream, Chromium's
  // `permissions.query` can still say `prompt`, because the stream was released again.
  const [asked, setAsked] = useState<{ mic?: MicState; notify?: MicState } | null>(null);
  const [busy, setBusy] = useState<"mic" | "notify" | null>(null);
  const [devices, setDevices] = useState<MediaDeviceInfo[] | null>(null);

  const mic = asked?.mic ?? probe.data?.mic ?? "unknown";
  const notify = asked?.notify ?? probe.data?.notify ?? "unknown";
  const inputs = devices ?? probe.data?.devices ?? [];

  const recheck = () => {
    setAsked(null);
    setDevices(null);
    probe.reload();
  };

  const askMic = async () => {
    setBusy("mic");
    try {
      const state = await requestMic();
      setAsked((current) => ({ ...current, mic: state }));
      setDevices(await inputDevices());
    } finally {
      setBusy(null);
    }
  };

  const askNotify = async () => {
    setBusy("notify");
    try {
      const state = await requestNotifications();
      setAsked((current) => ({ ...current, notify: state }));
    } finally {
      setBusy(null);
    }
  };

  const named = inputs.filter((device) => device.label.trim().length > 0);
  const system = systemAudio(platform);

  return (
    <section
      className={cn(
        "border-line bg-bg-raised rounded-[var(--radius-card)] border p-5 shadow-[var(--shadow-card)]",
        // Each row draws a rule above itself to separate it from the one before. With no heading
        // there is nothing before the first one, and the rule becomes a line across the top of the
        // card for no reason.
        compact &&
          "[&>div:first-of-type]:mt-0 [&>div:first-of-type]:border-t-0 [&>div:first-of-type]:pt-0",
      )}
    >
      {/* Setup puts its own numbered heading above this panel, and two headings saying the same
          thing one line apart is how a screen reads as assembled rather than designed. Settings has
          no such heading, so there it stays. */}
      {!compact && (
        <h2 className="text-body font-semibold tracking-tight">{t("permissions.title")}</h2>
      )}

      <Row
        testId="permission-mic"
        title={t("permissions.mic_title")}
        why={t("permissions.why")}
        state={mic}
        askLabel={t("permissions.ask")}
        busy={busy === "mic"}
        onAsk={() => void askMic()}
        onRecheck={recheck}
        steps={micRecovery(platform, browser)}
        platform={platform}
        onPlatform={setChosen}
      >
        {mic === "granted" && (
          <p className="text-fg-dim text-meta mt-2">
            {named[0]
              ? t("permissions.found", { count: String(inputs.length), name: named[0].label })
              : t("permissions.found_unnamed", { count: String(inputs.length) })}
          </p>
        )}
      </Row>

      {/* Notifications are optional, so they are second and quieter — but they are here at all
          because the code that asks for them existed for months with nothing calling it: the
          comment said "asking is a deliberate action in Settings" and Settings had no such action,
          so nudges could never notify anyone. */}
      {!compact && (
        <Row
          testId="permission-notify"
          title={t("permissions.notify_title")}
          why={t("permissions.notify_why")}
          state={notify}
          askLabel={t("permissions.notify_ask")}
          busy={busy === "notify"}
          onAsk={() => void askNotify()}
          onRecheck={recheck}
          steps={notificationRecovery(platform, browser)}
          platform={platform}
          onPlatform={setChosen}
          unsupported={t("permissions.notify_unsupported")}
        />
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

/** One permission: what it is for, what it is now, and what to do about it. */
function Row({
  testId,
  title,
  why,
  state,
  askLabel,
  busy,
  onAsk,
  onRecheck,
  steps,
  platform,
  onPlatform,
  unsupported,
  children,
}: {
  /** So a browser test can address one permission rather than counting lists on the page. */
  testId: string;
  title: string;
  why: string;
  state: MicState;
  askLabel: string;
  busy: boolean;
  onAsk: () => void;
  onRecheck: () => void;
  steps: Step[];
  platform: Platform;
  onPlatform: (platform: Platform) => void;
  /** Shown instead of a button where the browser has no such API at all. */
  unsupported?: string;
  children?: ReactNode;
}) {
  const t = useT();
  // `unknown` means the browser could not say. For the microphone that is normal — Safari does not
  // implement `permissions.query` — so the button still works. For notifications it means the API
  // is absent, and a button that cannot do anything is worse than a sentence saying so.
  const missing = unsupported !== undefined && state === "unknown";

  return (
    <div data-testid={testId} className="border-line mt-5 border-t pt-4 first-of-type:mt-4">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="text-meta font-medium">{title}</h3>
          <p className="text-fg-dim text-meta mt-1">{why}</p>
        </div>
        <Badge state={state} />
      </div>

      {children}

      {missing && <p className="text-fg-faint text-micro mt-3">{unsupported}</p>}

      {!missing && state !== "granted" && (
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button onClick={onAsk} disabled={busy}>
            {busy ? t("permissions.asking") : askLabel}
          </Button>
          <Button variant="ghost" onClick={onRecheck}>
            {t("permissions.recheck")}
          </Button>
        </div>
      )}

      {/* Steps only once refused. Showing the repair path to somebody who has not been asked yet
          makes a first run look like a problem. */}
      {state === "denied" && (
        <>
          <ol className="text-fg-dim text-meta mt-4 list-decimal space-y-2 pl-5">
            {steps.map((step) => (
              <li key={step.key}>{t(step.key, step.values)}</li>
            ))}
          </ol>
          <p className="text-fg-faint text-micro mt-3">
            {t("permissions.wrong_os")}{" "}
            {(["macos", "windows", "linux"] as const)
              .filter((other) => other !== platform)
              .map((other) => (
                <button
                  key={other}
                  type="button"
                  onClick={() => onPlatform(other)}
                  className="mr-2 underline"
                >
                  {t(`permissions.os_${other}`)}
                </button>
              ))}
          </p>
        </>
      )}
    </div>
  );
}

/** The current answer, in one word and one colour. */
function Badge({ state }: { state: MicState }) {
  const t = useT();
  const styles: Record<MicState, string> = {
    granted: "bg-accent-soft text-done border-accent/30",
    denied: "bg-rec-soft text-rec border-rec/30",
    prompt: "bg-bg-soft text-fg-faint border-line",
    unknown: "bg-bg-soft text-fg-faint border-line",
  };
  return (
    <span className={`text-micro shrink-0 rounded-full border px-2 py-1 ${styles[state]}`}>
      {t(`permissions.state_${state}`)}
    </span>
  );
}
