import { Page, Skeleton } from "../ui";

/**
 * The shape of a screen that has not arrived yet.
 *
 * Screens are fetched when they are opened (see `router.tsx`), and from a local daemon that takes
 * a few milliseconds — so this is almost never seen, and the router is configured not to show it
 * for a load that finishes quickly. What it is for is the case where it is seen: a cold cache on a
 * phone, or a shell whose assets are still being unpacked. The alternative in that case is a blank
 * pane, which reads as an app that has crashed rather than one that is a moment behind.
 *
 * Blocks the size of a heading and three cards, in the same frame every screen uses, so nothing
 * moves sideways when the real thing replaces it. `aria-busy` rather than a live region: a person
 * using a screen reader should hear the screen when it arrives, not an announcement that it is
 * coming.
 */
export function ScreenPending() {
  return (
    <div aria-busy="true">
      <Page>
        <Skeleton className="h-9 w-56" />
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          <Skeleton className="h-36" />
          <Skeleton className="h-36" />
          <Skeleton className="h-36" />
        </div>
      </Page>
    </div>
  );
}
