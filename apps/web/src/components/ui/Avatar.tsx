import { cn } from "../../lib/cn";
import { hueOf, initials } from "../../lib/avatar";

/**
 * A speaker, as a coloured disc with their initials.
 *
 * Summo has no photographs and will not have any — it never asks for an account, so there is
 * nowhere a face could come from. What it does have is a name, and a name is enough to make a
 * transcript scannable: the eye finds "NG" in a column of discs faster than it finds "Ngọc" in a
 * column of words, because colour and shape arrive before reading does.
 *
 * The colour is a hash of the name rather than a stored field. That means it survives a vault
 * copied to another machine, it needs no migration, and two people who sit in the same meetings get
 * different discs as long as their names differ — which is the only case that matters. It also
 * means renaming somebody recolours them, which is the honest behaviour: the disc is a picture of
 * the name, not an identity.
 *
 * Never the sole carrier of who is speaking. The name is always beside it, because a hue is not
 * readable to everyone and initials collide.
 */
export function Avatar({
  name,
  size = "md",
  className,
}: {
  name: string;
  size?: "sm" | "md";
  className?: string;
}) {
  const hue = hueOf(name);

  return (
    <span
      aria-hidden="true"
      className={cn(
        "grid shrink-0 place-items-center rounded-full font-semibold",
        // The discs were `text-[0.6rem]` and `text-[0.7rem]` — 9.6px and 11.2px, both written as
        // raw sizes because neither is a step on any scale. The smaller one was below anything
        // else in the interface, which is how two letters end up unreadable on a disc built to be
        // read at a glance.
        size === "sm" ? "text-micro size-5" : "text-meta size-7",
        className,
      )}
      style={{
        // `oklch` rather than `hsl`: at a fixed lightness the hues stay equally bright, so no
        // speaker gets a disc that reads as darker than the rest for no reason. Only the hue is
        // computed here — lightness and chroma come from `theme.css`, because they are what has to
        // change between a dark page and a light one.
        backgroundColor: `oklch(var(--avatar-fill-l) var(--avatar-fill-c) ${hue} / var(--avatar-fill-a))`,
        color: `oklch(var(--avatar-text-l) var(--avatar-text-c) ${hue})`,
      }}
    >
      {initials(name)}
    </span>
  );
}
