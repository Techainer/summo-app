/**
 * What the menu can do, in one list, so the two menus cannot drift.
 *
 * There are two of them and there has to be. macOS hangs its menu bar off the top of the *screen*,
 * so the native one — built in `apps/desktop/src-tauri/src/main.rs` — is the right one there, and a
 * second bar drawn inside the window would be a duplicate of the system's. Windows and Linux hang
 * it off the window frame, and this window is `decorations: false`: there is no frame, so the
 * native menu is built and never appears. The app draws its own there.
 *
 * Both of them run the same ids through the same handler. This file is the list, and it is what
 * makes "the Windows menu is missing an item" a thing that cannot happen quietly.
 */

export interface MenuAction {
  id: string;
  labelKey: string;
  /** Written as it is shown, `mod` standing in for ⌘ or Ctrl. */
  keys?: string[];
}

export interface MenuGroup {
  /** Matches the submenu titles the shell builds, so the two bars read the same. */
  labelKey: string;
  items: (MenuAction | "separator")[];
}

export const MENU: MenuGroup[] = [
  {
    labelKey: "menu.file",
    items: [
      { id: "new-note", labelKey: "menu.new_note", keys: ["mod", "N"] },
      { id: "import", labelKey: "menu.import", keys: ["mod", "O"] },
      { id: "record", labelKey: "menu.record", keys: ["mod", "⇧", "R"] },
      "separator",
      { id: "vault", labelKey: "menu.vault" },
    ],
  },
  {
    labelKey: "menu.view",
    items: [
      { id: "home", labelKey: "menu.home", keys: ["mod", "1"] },
      { id: "library", labelKey: "menu.library", keys: ["mod", "2"] },
      { id: "tasks", labelKey: "menu.tasks", keys: ["mod", "3"] },
      { id: "analytics", labelKey: "menu.analytics", keys: ["mod", "4"] },
      "separator",
      { id: "search", labelKey: "menu.search", keys: ["mod", "K"] },
      { id: "sidebar", labelKey: "menu.sidebar", keys: ["mod", "B"] },
      { id: "settings", labelKey: "menu.settings", keys: ["mod", ","] },
    ],
  },
  {
    labelKey: "menu.help",
    items: [
      { id: "shortcuts", labelKey: "menu.shortcuts", keys: ["?"] },
      { id: "docs", labelKey: "menu.docs" },
      { id: "issue", labelKey: "menu.issue" },
    ],
  },
];

/** Where **Help → Documentation** and **Report a problem** go. */
export const DOCS = "https://github.com/Techainer/summo-app#readme";
export const ISSUES = "https://github.com/Techainer/summo-app/issues/new";

/**
 * Edit is deliberately absent from the in-window bar.
 *
 * On macOS those items are the system's own — `PredefinedMenuItem::cut` and friends — and they are
 * the reason the native menu exists at all: without an Edit menu, ⌘C and ⌘Z are not bound inside a
 * webview. On Windows and Linux the webview binds Ctrl+C and Ctrl+Z itself, so a menu drawn in the
 * window would list commands it cannot perform. A menu item that does nothing is worse than one
 * that is not there.
 */
export const EDIT_IS_NATIVE_ONLY = true;
