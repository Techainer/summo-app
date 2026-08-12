import { type ClassValue, clsx } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

/**
 * The type scale from `theme.css`, declared to `tailwind-merge` as sizes.
 *
 * It cannot know: `text-meta` could be a size or a colour, and its guess is colour. So
 * `cn("text-accent-fg", "text-meta")` deleted the colour as a conflict, and every primary button
 * lost its foreground — dark text on green became the page's ordinary light text on green, at 1.39:1
 * contrast. `shots.mjs` caught it; nothing else could, because the class list is *right* in the
 * source and wrong only after merging.
 *
 * Anything added to `--text-*` in `theme.css` belongs here too.
 */
const SIZES = ["display", "title", "body", "meta", "micro"];

const merge = extendTailwindMerge({
  extend: { classGroups: { "font-size": [{ text: SIZES }] } },
});

/**
 * Join class names, letting later ones win over earlier ones.
 *
 * Without the merge step a component's own `px-3` and a caller's `px-6` both end up in the class
 * list and the winner depends on stylesheet order rather than on who asked last — which makes
 * overriding a component from outside unpredictable.
 */
export function cn(...inputs: ClassValue[]): string {
  return merge(clsx(inputs));
}
