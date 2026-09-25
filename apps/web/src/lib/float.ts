/**
 * A window that stays above the others.
 *
 * Document Picture-in-Picture: an operating-system window whose contents are ordinary DOM, so a
 * React tree can be portalled into it. That is what makes a minimised Summo useful rather than
 * decorative — a panel pinned inside the page goes behind the call it is captioning the moment
 * somebody clicks the call.
 *
 * Chromium has it; WebKit does not, which means it is present in a browser and in the Windows
 * desktop shell and absent in the macOS and Linux ones. [`canFloat`] is how the caller finds out
 * before offering something it cannot do.
 *
 * **Styles have to be carried across.** A new window starts with an empty document and none of this
 * page's CSS. Without copying it the panel renders as unstyled text on white, which is worse than
 * not opening — so the stylesheets are cloned, and a `<link>` is waited for, because a portal that
 * paints before its CSS arrives is a flash of exactly that.
 */

/** The bits of the API this uses, which TypeScript's DOM library does not describe yet. */
interface PictureInPicture {
  requestWindow: (options?: { width?: number; height?: number }) => Promise<Window>;
  window: Window | null;
}

function api(): PictureInPicture | null {
  const found = (globalThis as { documentPictureInPicture?: PictureInPicture })
    .documentPictureInPicture;
  return found && typeof found.requestWindow === "function" ? found : null;
}

/** Whether a real floating window can be opened here. */
export function canFloat(): boolean {
  return api() !== null;
}

export interface FloatingWindow {
  /** Portal target. */
  document: Document;
  /** Close it from this side. */
  close: () => void;
  /** Called when it closes from the other side — the user's own close button. */
  onClose: (handler: () => void) => void;
}

/**
 * Open one, or return `null` when the browser declines.
 *
 * `null` rather than a throw: refusing is an ordinary outcome — one such window is allowed at a
 * time and it must come from a user gesture — and the caller's answer to all of those is the same
 * fallback.
 */
export async function openFloatingWindow(size: {
  width: number;
  height: number;
}): Promise<FloatingWindow | null> {
  const pip = api();
  if (!pip) return null;

  let opened: Window;
  try {
    opened = await pip.requestWindow(size);
  } catch {
    return null;
  }

  await copyStyles(opened.document);

  return {
    document: opened.document,
    close: () => opened.close(),
    onClose: (handler) => opened.addEventListener("pagehide", handler, { once: true }),
  };
}

/**
 * Clone this document's styles into another one.
 *
 * Both kinds. A development build serves `<style>` elements Vite injected, a production build
 * serves a `<link>` — copying only one of them means the panel is styled in exactly one of the two
 * places anybody looks at it.
 */
async function copyStyles(target: Document): Promise<void> {
  const waits: Promise<unknown>[] = [];

  for (const node of document.querySelectorAll("style")) {
    target.head.append(node.cloneNode(true));
  }

  for (const node of document.querySelectorAll<HTMLLinkElement>('link[rel="stylesheet"]')) {
    const copy = target.createElement("link");
    copy.rel = "stylesheet";
    copy.href = node.href;
    waits.push(
      new Promise((resolve) => {
        copy.addEventListener("load", resolve, { once: true });
        // A stylesheet that will not load must not hold the panel closed forever.
        copy.addEventListener("error", resolve, { once: true });
      }),
    );
    target.head.append(copy);
  }

  // Every attribute of `<html>`, not a list of the ones known today.
  //
  // The theme is `data-theme` — `theme.css` has `:root[data-theme="dark"]` — and copying `class`,
  // `lang` and `dir` by name missed it: a window opened by somebody who had explicitly chosen dark
  // came up white, which is the one moment they had asked to be unobtrusive. Copying the lot means
  // the next attribute that carries state does not have to be remembered here.
  for (const attribute of document.documentElement.attributes) {
    target.documentElement.setAttribute(attribute.name, attribute.value);
  }

  await Promise.all(waits);
}
