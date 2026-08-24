// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";

import {
  active,
  adopt,
  apply,
  choose,
  read,
  resolved,
  stored,
  subscribe,
  STORAGE_KEY,
  type Scheme,
} from "./theme";

/**
 * The theme, and the reason this file exists at all.
 *
 * `theme.css` has defined `:root[data-theme="light"]` and `:root[data-theme="dark"]` since the
 * palette was rebuilt, and nothing set the attribute — two fully-written blocks of CSS that no user
 * could ever reach. These tests are cheap; the bug they guard against is invisible.
 */
describe("the theme preference", () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  it("follows the machine until somebody says otherwise", () => {
    expect(read()).toBe("system");
  });

  it("remembers a choice and puts it on the document", () => {
    choose("dark");
    expect(read()).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  /**
   * `system` removes the attribute rather than writing the current OS value into it. Otherwise a
   * user who switches their laptop to dark at sunset would have to come back here and say so again.
   */
  it("goes back to following the machine rather than freezing today's answer", () => {
    choose("light");
    choose("system");
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
    expect(read()).toBe("system");
  });

  it("ignores a stored value that is not a scheme", () => {
    window.localStorage.setItem(STORAGE_KEY, "midnight");
    expect(read()).toBe("system");
  });

  it("reports what is actually being painted, not what was asked for", () => {
    expect(resolved("dark")).toBe("dark");
    // jsdom answers `false` to every media query, which is the light branch.
    expect(resolved("system")).toBe("light");
  });

  it("applies without remembering, for a preview", () => {
    apply("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    expect(window.localStorage.getItem(STORAGE_KEY)).toBeNull();
  });
});

describe("the vault's copy of the choice", () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
  });

  /**
   * The visit this exists for: a second machine, or a reinstall, with a preference set elsewhere.
   * Before the vault's copy was read, every browser started from `system` however many times the
   * user had said otherwise.
   */
  it("takes the vault's theme when this browser has never been told one", () => {
    expect(adopt("dark")).toBe("dark");
    expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
  });

  /**
   * And loses to a local choice, which is what keeps the browser authoritative for what is on
   * screen. The vault's answer arrives after first paint; letting it win would undo a choice the
   * user had just made in this window, a second after making it.
   */
  it("leaves a browser that has chosen alone", () => {
    choose("light");
    expect(adopt("dark")).toBe(null);
    expect(document.documentElement.getAttribute("data-theme")).toBe("light");
  });

  /**
   * `system` is a choice somebody made, not an empty slot. Reading the two as the same value is
   * precisely why this could not work: every browser looked as though it had already decided.
   */
  it("treats a stored `system` as a decision", () => {
    choose("system");
    expect(stored()).toBe("system");
    expect(adopt("dark")).toBe(null);
  });

  /** A daemon that answers with nothing, or with junk, must not paint anything. */
  it("ignores an answer that is not a scheme", () => {
    expect(adopt(undefined)).toBe(null);
    expect(adopt("")).toBe(null);
    expect(adopt("neon")).toBe(null);
    expect(document.documentElement.hasAttribute("data-theme")).toBe(false);
  });

  /**
   * Adopting must not write. Otherwise the first visit turns "the vault prefers dark" into "this
   * browser has decided dark", and the next change made on another machine stops arriving.
   */
  it("does not claim the choice as this browser's own", () => {
    adopt("dark");
    expect(stored()).toBe(null);
  });
});

/**
 * What the controls read, which is not what storage holds.
 *
 * Three components showed the scheme and each kept its own `useState(read())`. The header button
 * and the segmented control in Cài đặt → Chung are on screen together, so changing the theme in one
 * left the other displaying the previous answer — and neither could hear `adopt`, so the visit the
 * vault's copy exists for painted dark under a control still reading "Hệ thống".
 */
describe("what is on screen", () => {
  beforeEach(() => {
    window.localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    // The module keeps the live answer; put it back where a fresh browser starts.
    choose("system");
    window.localStorage.clear();
  });

  it("follows a choice made anywhere", () => {
    const seen: Scheme[] = [];
    const stop = subscribe(() => seen.push(active()));

    choose("dark");
    expect(active()).toBe("dark");
    // Adopting paints, so it must report too, even though it writes nothing down.
    window.localStorage.clear();
    adopt("light");
    expect(active()).toBe("light");

    stop();
    choose("dark");
    expect(seen).toEqual(["dark", "light"]);
  });

  /** Nothing changed is not a change: a repeated choice must not re-render every watcher. */
  it("stays quiet when the answer is the same", () => {
    choose("dark");
    let told = 0;
    const stop = subscribe(() => told++);
    choose("dark");
    expect(told).toBe(0);
    stop();
  });
});
