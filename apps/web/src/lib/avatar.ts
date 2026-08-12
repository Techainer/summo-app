/**
 * Turning a name into a disc: the letters, and the colour.
 *
 * Separate from the component so the rules can be tested as functions and so the module that draws
 * a person exports only a component. Both are pure and both depend on nothing but the name.
 */
/**
 * One or two letters, upper-cased.
 *
 * The *last* two words, not the first two: Vietnamese names put the given name last, and "Nguyễn
 * Thị Ngọc" is Ngọc to everyone who knows her. Taking the first two would label half the country
 * "NT".
 */
export function initials(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  const last = words.slice(-2);
  return last
    .map((word) => [...word][0] ?? "")
    .join("")
    .toLocaleUpperCase();
}

/** A stable hue in [0, 360) from the name. FNV-1a, over code points so CJK and Vietnamese hash. */
export function hueOf(name: string): number {
  let hash = 0x811c9dc5;
  for (const point of name.trim().toLocaleLowerCase()) {
    hash ^= point.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash % 360;
}
