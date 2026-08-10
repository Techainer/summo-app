import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Join class names, letting later ones win over earlier ones.
 *
 * Without the merge step a component's own `px-3` and a caller's `px-6` both end up in the class
 * list and the winner depends on stylesheet order rather than on who asked last — which makes
 * overriding a component from outside unpredictable.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
