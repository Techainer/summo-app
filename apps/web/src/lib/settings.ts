/**
 * Which parts Settings is divided into.
 *
 * Here rather than beside the component because the router validates `?section=` against it, and
 * the router is in the entry chunk: importing the settings *component* to get one string union
 * would pull the whole screen — every field, the storage panel, the about box — back into the
 * bundle that loads before anything is drawn. That is the regression the lazy routes were for.
 */
export const SECTION_IDS = [
  "general",
  "recording",
  "ai",
  "translation",
  "storage",
  "about",
] as const;

export type SectionId = (typeof SECTION_IDS)[number];

/** An unknown or absent section opens the first one; a hand-edited URL must not blank the screen. */
export function isSection(value: unknown): value is SectionId {
  return typeof value === "string" && (SECTION_IDS as readonly string[]).includes(value);
}
