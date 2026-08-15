import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * The shells' content policy permits every kind of request this interface actually makes.
 *
 * This exists because of a failure that is invisible to every other check in the repository. In a
 * browser the daemon serves the interface itself, so its port is the page's own origin and the
 * policy is irrelevant — all twenty browser suites pass no matter what it says. In the desktop and
 * mobile apps the page is loaded from the bundle over a custom scheme, and `http://127.0.0.1:<port>`
 * is then a *different* origin that has to be named, directive by directive.
 *
 * It was named for `connect-src` only. `fetch` and the WebSocket worked; every picture in a note
 * and every recording played back were blocked by `default-src 'self'`, in the packaged apps and
 * nowhere else. Nothing failed loudly: a blocked `<img>` is a broken picture and a blocked
 * `<audio>` is a player that will not start.
 *
 * So the rule below is not "the policy contains this string" but "every directive the interface
 * relies on names the daemon" — because the next thing added will be a `<video>` or a font, and it
 * will be added by someone who has only ever run this in a browser.
 */

const DAEMON = "http://127.0.0.1:*";

/**
 * The directives the interface depends on, and what makes each one load from the daemon.
 *
 * Kept as prose rather than a bare list so that removing one is a decision rather than a tidy-up.
 */
const REQUIRED: Record<string, string> = {
  // `LibraryClient` and everything built on it, plus the transcript socket.
  "connect-src": "fetch and the WebSocket",
  // `NoteClient.src` — a picture stored in `vault/attachments/`, served by the daemon.
  "img-src": "pictures in notes",
  // `Player` — the lanes of a recording, streamed from the daemon.
  "media-src": "listening back to a meeting",
};

const config = (relative: string) =>
  JSON.parse(readFileSync(fileURLToPath(new URL(relative, import.meta.url)), "utf8")) as {
    app: { security: { csp: string } };
  };

/** `img-src 'self' data: …` → `["'self'", "data:", …]`. */
function directives(csp: string): Map<string, string[]> {
  return new Map(
    csp
      .split(";")
      .map((part) => part.trim())
      .filter((part) => part !== "")
      .map((part) => {
        const [name, ...sources] = part.split(/\s+/);
        return [name!, sources] as const;
      }),
  );
}

const SHELLS = {
  desktop: "../../../../apps/desktop/src-tauri/tauri.conf.json",
  mobile: "../../../../apps/mobile/src-tauri/tauri.conf.json",
};

describe.each(Object.entries(SHELLS))("the %s shell's content policy", (_shell, path) => {
  const csp = config(path).app.security.csp;
  const found = directives(csp);

  it.each(Object.entries(REQUIRED))("lets %s reach the daemon (%s)", (directive) => {
    expect(
      found.get(directive),
      `${directive} is absent, so it falls back to default-src and the daemon is blocked`,
    ).toBeDefined();
    expect(found.get(directive)).toContain(DAEMON);
  });

  it("keeps the daemon on loopback", () => {
    // A wildcard host would let any site the webview can be pointed at reach the microphone's
    // daemon. Everything Summo talks to is on this machine, by design.
    for (const sources of found.values()) {
      for (const source of sources) {
        expect(source, "only loopback and the bundle itself").toMatch(
          /^(?:'self'|'unsafe-inline'|data:|(?:https?|ws):\/\/127\.0\.0\.1:\*)$/,
        );
      }
    }
  });
});

it("gives both shells the same policy", () => {
  // One interface, two shells. A directive added to one and not the other is a feature that works
  // on a laptop and not on a phone, discovered by whoever has the phone.
  expect(config(SHELLS.desktop).app.security.csp).toBe(config(SHELLS.mobile).app.security.csp);
});
