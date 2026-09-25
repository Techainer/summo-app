/**
 * Every screen the app has, and the ones deliberately left out.
 *
 * One list, read by everything that walks the app. It lived inside `shots.mjs`, which meant the
 * next suite that wanted "all the screens" would have copied it — and a copied list is a list that
 * goes one screen out of date the first time somebody adds a route and updates the other one.
 *
 * The completeness check travels with it: a route declared in `src/router.tsx` that appears in
 * neither list fails the suite that asked. That rule was learned the hard way — the one screen with
 * a transcript on it sat behind a `null` and a comment for months, so the sideways-scroll check ran
 * over eleven screens of short text and never looked at the place a recogniser's output lands.
 */
import { readFileSync } from "node:fs";

/**
 * The settings sections, read out of the source.
 *
 * Settings is one route and seven screens. Walking `/settings` alone touched the twenty-one
 * controls of whichever section opens first and none of the rest — so a panel could render blank,
 * scroll sideways on a phone, or ship a nameless button, and every suite here would pass. `sync`
 * landed exactly that way.
 *
 * Parsed rather than listed, because a list written here is a list that goes stale the first time
 * somebody adds a section — which is the same lesson this file exists to enforce for routes.
 */
function settingsSections() {
  const source = readFileSync(new URL("../src/lib/settings.ts", import.meta.url), "utf8");
  const block = /SECTION_IDS = \[([\s\S]*?)\] as const/.exec(source);
  if (!block) {
    console.error("cannot find SECTION_IDS in src/lib/settings.ts — has it moved?");
    process.exit(1);
  }
  const found = [...block[1].matchAll(/"([a-z-]+)"/g)].map((m) => m[1]);
  if (found.length < 5) {
    console.error(`only ${found.length} settings section(s) parsed — the shape changed.`);
    process.exit(1);
  }
  return found;
}

/** Routes worth walking, with the hash the router uses. `01E2E0` is what `seedVault` writes. */
export const SCREENS = [
  ["record", "/"],
  ["library", "/library"],
  ["meeting", "/pages/01E2E0"],
  ["notes", "/notes"],
  ["tasks", "/tasks"],
  ["agents", "/agents"],
  ["agenda", "/agenda"],
  ["chat", "/chat"],
  ["analytics", "/analytics"],
  ["people", "/people"],
  ["models", "/models"],
  ["help", "/help"],
  // One entry per settings section; see `settingsSections`. `/settings` itself is not listed
  // separately — it opens the first section, which is the first of these.
  ...settingsSections().map((id) => [`settings:${id}`, `/settings?section=${id}`]),
];

/** Routes left out on purpose, each with the reason. */
export const SKIPPED = new Map([
  ["/record", "redirects to /"],
  ["/meetings/$meetingId", "renders the page screen, already walked as /pages/01E2E0"],
  ["/__ui", "dev-only gallery, covered by ui-shots.mjs"],
]);

/**
 * Fail when a route exists and nothing walks it.
 *
 * `label` names the caller, so the message says which suite is blind rather than only that one is.
 */
export function everyRouteIsCovered(label) {
  const router = readFileSync(new URL("../src/router.tsx", import.meta.url), "utf8");
  const declared = [...router.matchAll(/^\s*path: "([^"]+)",/gm)].map((m) => m[1]);
  const covered = new Set(
    SCREENS.map(([, route]) =>
      // `?section=` is a section of one screen, not a route of its own — the router declares
      // `/settings` and nothing else, so the query is dropped before the comparison.
      route.replace(/\?.*$/, "").replace(/\/pages\/.*/, "/pages/$pageId"),
    ),
  );
  const missed = declared.filter((route) => !covered.has(route) && !SKIPPED.has(route));
  if (missed.length > 0) {
    console.error(
      `these routes are declared in router.tsx and ${label} reaches none of them: ${missed.join(", ")}\n` +
        "add them to SCREENS in e2e/screens.mjs, or to SKIPPED with the reason.",
    );
    process.exit(1);
  }
}
