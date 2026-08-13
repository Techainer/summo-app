/**
 * Asking for the microphone, and saying what to do when the answer is no.
 *
 * A recording app that cannot open the microphone is not partly broken, it is entirely broken, and
 * the operating system's own explanation is famously unhelpful: macOS says an app "would like to
 * access the microphone" once, ever, and if that prompt was dismissed it never appears again —
 * the app simply fails, and nothing on screen connects the failure to a checkbox three levels deep
 * in System Settings.
 *
 * So this module does three things the browser will not:
 *
 * 1. **Asks the state before asking for the device.** `getUserMedia` on a denied permission fails
 *    instantly with the same error as a missing microphone, so a user who refused once gets told
 *    to plug in a microphone they already have.
 * 2. **Names the exact place to fix it**, per operating system and per browser. "Grant it in your
 *    settings" is advice that assumes the reader already knew.
 * 3. **Keeps the request inside a click.** Browsers only show the prompt from a user gesture, and
 *    asking on load — before the app has shown what it is for — is how a permission gets refused
 *    permanently by a person who was only trying to look around.
 *
 * The instructions are keys rather than sentences: this file decides *which* advice applies, and
 * the locale files hold the words, because the advice differs by platform but the app is used in
 * four languages.
 */

/** Where the app is running, as far as the advice is concerned. */
export type Platform = "macos" | "windows" | "linux" | "unknown";

/** Which browser, when its own permission UI is what needs describing. */
export type Browser = "chrome" | "firefox" | "safari" | "other";

/**
 * What the operating system and browser currently think.
 *
 * `unknown` is a real answer, not a failure: Safari does not implement `permissions.query` for the
 * microphone, so the only way to learn the state there is to ask for the device. An interface that
 * treated unknown as denied would show a recovery panel to somebody who has granted nothing yet.
 */
export type MicState = "granted" | "denied" | "prompt" | "unknown";

/** A single instruction, as a locale key plus the values it interpolates. */
export interface Step {
  key: string;
  values?: Record<string, string>;
}

/**
 * The app this browser's permission lives under — what the user has to find in a list.
 *
 * Brand names, so they are the same in every language and are not in the locale files. `null` for
 * an unrecognised browser, which selects a phrasing that does not name one rather than a guess: a
 * user told to enable "Google Chrome" in a pane where only Brave appears stops reading.
 */
export function hostApp(browser: Browser): string | null {
  switch (browser) {
    case "chrome":
      return "Google Chrome";
    case "firefox":
      return "Firefox";
    case "safari":
      return "Safari";
    default:
      return null;
  }
}

/**
 * Detect the platform.
 *
 * `userAgentData.platform` where it exists — it is the only one of these that is not deprecated —
 * and the user-agent string otherwise. Both can be spoofed, and neither matters if they are: the
 * cost of guessing wrong is instructions for the wrong operating system, which is why the panel
 * also offers the other platforms rather than only the detected one.
 */
export function detectPlatform(nav: Navigator = navigator): Platform {
  const hinted = (nav as Navigator & { userAgentData?: { platform?: string } }).userAgentData
    ?.platform;
  const text = `${hinted ?? ""} ${nav.userAgent ?? ""}`.toLowerCase();
  if (text.includes("mac")) return "macos";
  if (text.includes("win")) return "windows";
  if (text.includes("linux") || text.includes("x11") || text.includes("android")) return "linux";
  return "unknown";
}

/** Detect the browser, for the half of the advice that is about the address bar. */
export function detectBrowser(nav: Navigator = navigator): Browser {
  const ua = (nav.userAgent ?? "").toLowerCase();
  // Order matters: every Chromium browser claims to be Safari, and Edge claims to be Chrome.
  if (ua.includes("firefox")) return "firefox";
  if (ua.includes("edg/") || ua.includes("chrome") || ua.includes("chromium")) return "chrome";
  if (ua.includes("safari")) return "safari";
  return "other";
}

/**
 * What the browser says about the microphone, without asking for it.
 *
 * Never throws: `permissions.query` rejects for unsupported names in some browsers and is absent
 * in others, and a permission *check* that breaks the screen it is on is worse than no check.
 */
export async function micState(nav: Navigator = navigator): Promise<MicState> {
  try {
    const status = await nav.permissions?.query({ name: "microphone" });
    if (!status) return "unknown";
    if (status.state === "granted" || status.state === "denied" || status.state === "prompt") {
      return status.state;
    }
    return "unknown";
  } catch {
    return "unknown";
  }
}

/**
 * Ask for the microphone, and release it immediately.
 *
 * The point is the permission, not the audio: holding the stream open would light the recording
 * indicator on a machine that is not recording, which is exactly the thing a local-first app must
 * never do.
 */
export async function requestMic(deviceId?: string): Promise<MicState> {
  try {
    const stream = await navigator.mediaDevices.getUserMedia({
      audio: deviceId ? { deviceId: { exact: deviceId } } : true,
    });
    stream.getTracks().forEach((track) => track.stop());
    return "granted";
  } catch (error) {
    // `NotAllowedError` is a refusal; anything else is a device problem, and the caller's error
    // text says which. Reporting a broken microphone as a refused permission would send the user
    // to a settings pane where everything is already correct.
    return error instanceof DOMException && error.name === "NotAllowedError" ? "denied" : "unknown";
  }
}

/**
 * Microphones this browser can see.
 *
 * Labels are empty until permission is granted — that is the specification, not a bug — so an empty
 * label is shown as "micro không tên" rather than hidden, because the count still tells a user
 * whether the machine has any input at all.
 */
export async function inputDevices(): Promise<MediaDeviceInfo[]> {
  try {
    const all = await navigator.mediaDevices.enumerateDevices();
    return all.filter((device) => device.kind === "audioinput");
  } catch {
    return [];
  }
}

/**
 * The steps that actually fix a refused microphone, in the order to do them.
 *
 * Browser first, operating system second, and that order is deliberate: the browser's own block is
 * both far more common and far easier to undo, and a user sent to System Settings for a permission
 * the OS already granted concludes the instructions are wrong and stops reading.
 */
export function micRecovery(platform: Platform, browser: Browser): Step[] {
  const steps: Step[] = [];

  switch (browser) {
    case "firefox":
      steps.push({ key: "permissions.fix_browser_firefox" });
      break;
    case "safari":
      steps.push({ key: "permissions.fix_browser_safari" });
      break;
    default:
      steps.push({ key: "permissions.fix_browser_chrome" });
  }

  switch (platform) {
    case "macos": {
      // The one that catches people: macOS grants microphone access to the *application*, and for
      // Summo that application is the browser — or the terminal, when the daemon was started from
      // one and the desktop shell is not in use.
      const app = hostApp(browser);
      steps.push(
        app
          ? { key: "permissions.fix_macos", values: { app } }
          : { key: "permissions.fix_macos_any" },
      );
      break;
    }
    case "windows":
      steps.push({ key: "permissions.fix_windows" });
      break;
    case "linux":
      steps.push({ key: "permissions.fix_linux" });
      break;
    default:
      break;
  }

  steps.push({ key: "permissions.fix_retry" });
  return steps;
}

/**
 * Whether this machine can capture what it is playing, and what to do if not.
 *
 * Half a meeting lives in the speakers, and the three platforms are genuinely different rather
 * than differently configured: Linux exposes a monitor source, Windows has WASAPI loopback, and
 * macOS has no route at all without either a virtual device or the Screen Recording permission —
 * which is a great deal to ask of an audio recorder, and is why Summo does not ask for it.
 */
export function systemAudio(platform: Platform): { supported: boolean; key: string } {
  switch (platform) {
    case "linux":
      return { supported: true, key: "permissions.system_linux" };
    case "windows":
      return { supported: true, key: "permissions.system_windows" };
    case "macos":
      return { supported: false, key: "permissions.system_macos" };
    default:
      return { supported: false, key: "permissions.system_unknown" };
  }
}

/**
 * Whether the browser will show a notification right now.
 *
 * Same three answers as the microphone, and the same reason for `unknown`: a browser without the
 * `Notification` API at all — an insecure context, an embedded webview — has no state to report,
 * and showing a repair path for a feature that cannot exist is worse than staying quiet.
 */
export function notificationState(): MicState {
  if (typeof Notification === "undefined") return "unknown";
  switch (Notification.permission) {
    case "granted":
      return "granted";
    case "denied":
      return "denied";
    default:
      return "prompt";
  }
}

/**
 * Ask for notifications, from a click.
 *
 * Nudges have been able to notify since they were written and never could in practice: the helper
 * that asks was never called from anywhere, because "asking is a deliberate action in Settings" and
 * Settings had no such action. This is that action.
 */
export async function requestNotifications(): Promise<MicState> {
  if (typeof Notification === "undefined") return "unknown";
  if (Notification.permission !== "default") return notificationState();
  const answer = await Notification.requestPermission();
  return answer === "granted" ? "granted" : answer === "denied" ? "denied" : "prompt";
}

/**
 * Fixing refused notifications, which is a different pane from the microphone on every platform.
 *
 * The operating-system step is not about permission at all on two of the three: macOS and Windows
 * will happily grant the permission and then swallow every notification because a Focus mode is on,
 * and a user who has granted access and still sees nothing has no way to guess that.
 */
export function notificationRecovery(platform: Platform, browser: Browser): Step[] {
  const steps: Step[] = [];

  switch (browser) {
    case "firefox":
      steps.push({ key: "permissions.notify_browser_firefox" });
      break;
    case "safari":
      steps.push({ key: "permissions.notify_browser_safari" });
      break;
    default:
      steps.push({ key: "permissions.notify_browser_chrome" });
  }

  switch (platform) {
    case "macos": {
      const app = hostApp(browser);
      steps.push(
        app
          ? { key: "permissions.notify_macos", values: { app } }
          : { key: "permissions.notify_macos_any" },
      );
      break;
    }
    case "windows":
      steps.push({ key: "permissions.notify_windows" });
      break;
    case "linux":
      steps.push({ key: "permissions.notify_linux" });
      break;
    default:
      break;
  }

  steps.push({ key: "permissions.fix_retry" });
  return steps;
}
