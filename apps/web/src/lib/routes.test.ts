import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * Every address this app asks the daemon for must be an address the daemon answers.
 *
 * Written after `/hardware` — a handler that has existed since the hardware probe landed, behind a
 * route registered as `/hw`. The status bar polled the wrong one, got the HTML shell back with a
 * cheerful `200`, parsed no memory out of it, and rendered nothing. Nothing failed anywhere: not
 * `tsc`, not clippy, not a single test, because both halves were individually correct.
 *
 * That is the recurring shape of every real bug in this repository — a capability granted to a
 * window nobody creates, a keyboard shortcut in a menu no platform draws, a route with no handler.
 * Two files that each look right and were never held against each other. So this holds them
 * against each other, at the only moment it is cheap: `pnpm test`, in under a second, with no
 * daemon and no browser.
 *
 * The comparison is deliberately blunt. Paths are normalised down to their shape — `${id}` and
 * `{id}` both become `{}` — because what is being checked is "does the daemon serve something at
 * this address", not the parameter's name. A false alarm costs one line here; the failure it
 * prevents shipped to a user twice.
 */

const HERE = new URL(".", import.meta.url).pathname;
const WEB = join(HERE, "..");
const SERVER = join(HERE, "../../../../crates/summo-engine/src/server.rs");

/** `.route("/onboarding/recommend", get(...))` → `/onboarding/recommend`. */
function daemonRoutes(): Set<string> {
  const source = readFileSync(SERVER, "utf8");
  const routes = new Set<string>();
  for (const match of source.matchAll(/\.route\(\s*"([^"]+)"/g)) {
    routes.add(shapeOf(match[1] ?? ""));
  }
  return routes;
}

/**
 * The paths the app asks for, and where it asks from.
 *
 * `url(handshake, "/x")` is the one way this app addresses the daemon — `lib/library.ts` builds
 * every request URL, because the token has to be attached to all of them — so a literal in that
 * position is the complete list.
 */
function clientCalls(): { path: string; file: string }[] {
  const calls: { path: string; file: string }[] = [];
  for (const file of sources(WEB)) {
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/\burl\(\s*[\w.]+\s*,\s*[`"]([^`"]+)[`"]/g)) {
      calls.push({ path: shapeOf(match[1] ?? ""), file: file.slice(WEB.length + 1) });
    }
  }
  return calls;
}

function sources(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir)) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      out.push(...sources(path));
    } else if (/\.tsx?$/.test(entry) && !entry.endsWith(".test.ts")) {
      out.push(path);
    }
  }
  return out;
}

/** `/notes/${id}` and `/notes/{id}` are the same address as far as this file is concerned. */
function shapeOf(path: string): string {
  return path
    .replace(/\$\{[^}]*\}/g, "{}")
    .replace(/\{[^}]*\}/g, "{}")
    .replace(/\/+$/, "");
}

/**
 * Whether a request the app makes could land on a route the daemon serves.
 *
 * Both sides have holes in them and the holes are not the same shape. The daemon's are always whole
 * segments — `{id}`. The app's can be part of one, because a path is built by interpolation:
 * `/meetings/${id}/comments${suffix}` is one address when the suffix is empty and another when it
 * is `/{comment}/react`. So a hole that fills a whole segment matches one segment, and a hole with
 * text beside it may swallow the rest of the path.
 *
 * The deliberate consequence: a fully static path — the case that has actually broken, twice — is
 * compared exactly, and nothing else has to be guessed at.
 */
function reaches(call: string, route: string): boolean {
  const wanted = call.split("/").filter(Boolean);
  const served = route.split("/").filter(Boolean);

  const walk = (i: number, j: number): boolean => {
    if (i === wanted.length) return j === served.length;
    const segment = wanted[i] ?? "";
    if (j === served.length) return false;

    // A whole segment interpolated, or a parameter on the daemon's side: one segment either way.
    if (segment === "{}" || (served[j] ?? "").startsWith("{")) return walk(i + 1, j + 1);

    if (segment.includes("{}")) {
      // Text and a hole in the same segment. The hole may be empty, one segment, or several — so
      // the prefix before it has to match, and everything after may be anything.
      const prefix = segment.slice(0, segment.indexOf("{}"));
      return (served[j] ?? "").startsWith(prefix);
    }

    return segment === served[j] && walk(i + 1, j + 1);
  };

  return walk(0, 0);
}

describe("the daemon's routes and the app's requests", () => {
  it("finds both sides to compare", () => {
    // A regex that silently matches nothing would make every assertion below pass. This is the
    // check that the checks are running at all — the same mistake that made an e2e suite report
    // four passing states against a screen rendering none of them.
    expect(daemonRoutes().size).toBeGreaterThan(40);
    expect(clientCalls().length).toBeGreaterThan(40);
  });

  it("serves every address the app asks for", () => {
    const routes = daemonRoutes();
    const served = [...routes];
    const missing = clientCalls()
      .filter((call) => !served.some((route) => reaches(call.path, route)))
      .map((call) => `${call.path} (${call.file})`);

    expect(missing, "the daemon has no handler at these addresses").toEqual([]);
  });
});
