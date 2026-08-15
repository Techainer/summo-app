// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from "vitest";

import { apply, choose, read, resolved, STORAGE_KEY } from "./theme";

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
