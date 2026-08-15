import {
  Outlet,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  lazyRouteComponent,
  redirect,
} from "@tanstack/react-router";

import { claimHandshake } from "./lib/session";
import { RootLayout } from "./components/shell/RootLayout";
import { ScreenPending } from "./components/shell/ScreenPending";
import { HomeScreen } from "./screens/HomeScreen";

/**
 * Every screen but the first one, fetched when it is opened.
 *
 * All thirteen used to be imported at the top of this file, which put all thirteen into the chunk
 * the browser has to parse before it can paint anything — the models catalogue, the analytics
 * charts and the settings form included, on an app that opens on Home. It was 96 kB gzipped of
 * entry chunk and a first load of 277 kB against a 300 kB budget, most of it screens nobody had
 * asked for yet.
 *
 * Home stays eager on purpose: splitting the screen the app starts on trades a smaller parse for a
 * second round trip before the first pixel, which is the wrong way round.
 *
 * The rest are warmed on idle by {@link warmScreens} rather than left to be fetched on the click.
 * From a local daemon or a `tauri://` bundle a chunk arrives in single-digit milliseconds, so this
 * is not about the network — it is about *when* the browser spends the parse: after the first
 * screen is on the glass instead of before it.
 */
const screens = {
  record: () => import("./screens/RecordScreen"),
  library: () => import("./screens/LibraryScreen"),
  page: () => import("./screens/PageScreen"),
  notes: () => import("./screens/NotesScreen"),
  agenda: () => import("./screens/AgendaScreen"),
  chat: () => import("./screens/ChatScreen"),
  agents: () => import("./screens/AgentsScreen"),
  tasks: () => import("./screens/TasksScreen"),
  people: () => import("./screens/PeopleScreen"),
  analytics: () => import("./screens/AnalyticsScreen"),
  models: () => import("./screens/ModelsScreen"),
  settings: () => import("./screens/SettingsScreen"),
};

/**
 * Fetch every screen's chunk, for when the browser has nothing better to do.
 *
 * Called from `main.tsx` on idle. Without it the first visit to each screen pays a fetch at the
 * moment somebody is watching, which is the one moment it is expensive; with it navigation is as
 * instant as it was when everything was in one file, and the first paint is not waiting on any of
 * it. Failures are ignored — a chunk that could not be prefetched is fetched again by the router
 * when the screen is actually opened, and reporting it here would be an error about a screen the
 * user has not asked for.
 */
export function warmScreens() {
  for (const fetch of Object.values(screens)) void fetch().catch(() => undefined);
}

/**
 * Hash history, not browser history.
 *
 * The desktop shell serves the built assets from a custom scheme (`tauri://`) with a relative base,
 * where pushing real paths asks the webview for a document that does not exist. A hash keeps every
 * route inside one document while still giving real URLs, back/forward, and deep links into a
 * meeting.
 */
// Before the history is created, because creating it parses the URL once and immediately — and the
// thing being removed is what makes that parse wrong. See `claimHandshake`.
claimHandshake();

const history = createHashHistory();

export interface LibrarySearch {
  /** A folder path. `""` is the vault root — the things nobody has filed — not "no filter". */
  folder?: string;
  /** Comma-separated. A document must carry every one of them. */
  tag?: string;
  /** A palette name; see `crates/summo-vault/src/colour.rs`. */
  colour?: string;
  q?: string;
}

const rootRoute = createRootRoute({
  component: () => (
    <RootLayout>
      <Outlet />
    </RootLayout>
  ),
});

const homeRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: HomeScreen,
});

/// Recording keeps a destination of its own.
///
/// It used to be the landing page, which was an honest picture of a recorder and a poor one of a
/// workspace. It is still the one irreversible action, so it keeps a route, a sidebar item and a
/// header button — moving it into a card on Home would have made the thing you cannot undo the
/// thing you have to hunt for.
const recordRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/record",
  component: lazyRouteComponent(screens.record, "RecordScreen"),
});

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: lazyRouteComponent(screens.library, "LibraryScreen"),
  // Filters live in the URL so a filtered view can be reloaded, shared and stepped back out of.
  // Every key is optional: `/library` with no query is the unfiltered view, and callers should be
  // able to set one filter without restating the others.
  validateSearch: (search: Record<string, unknown>): LibrarySearch => {
    const text = (value: unknown) =>
      typeof value === "string" && value !== "" ? value : undefined;
    // The folder is the exception: an empty string is the vault root, which is a real folder to
    // browse and the one people reach for most, since it is everything they have not filed yet.
    const folder = typeof search.folder === "string" ? search.folder : undefined;
    return {
      folder,
      tag: text(search.tag),
      colour: text(search.colour),
      q: text(search.q),
    };
  },
});

/// One address for a document, whichever kind it is.
///
/// A note used to open at `/notes?open=<id>` and a recording at `/meetings/<id>`, so everything
/// that can point at a document had to know which it was pointing at — and five call sites got it
/// wrong the same way, sending notes to a screen with nothing open. The vault has only ever had one
/// kind of document; this is its address.
const pageRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/pages/$pageId",
  component: lazyRouteComponent(screens.page, "PageScreen"),
});

/// Kept, as a redirect. `/meetings/<id>` is in links people have saved, in the desktop shell's
/// deep-link handling and in every citation written into the vault before today.
const meetingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/meetings/$meetingId",
  beforeLoad: ({ params }) => {
    // Throwing is how the router is told to redirect, and what it wants thrown is a plain
    // descriptor rather than an `Error`. The rule is right about ordinary code and wrong about this
    // one control-flow protocol.
    // eslint-disable-next-line @typescript-eslint/only-throw-error
    throw redirect({ to: "/pages/$pageId", params: { pageId: params.meetingId } });
  },
});

const notesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/notes",
  component: lazyRouteComponent(screens.notes, "NotesScreen"),
  // `?open=` so a note is a link. Without it the sidebar could list every page and open none of
  // them, which is a table of contents for a book with no page numbers.
  validateSearch: (search: Record<string, unknown>) => ({
    open: typeof search.open === "string" && search.open ? search.open : undefined,
  }),
});

const agendaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agenda",
  component: lazyRouteComponent(screens.agenda, "AgendaScreen"),
});

const chatRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/chat",
  component: lazyRouteComponent(screens.chat, "ChatScreen"),
});

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: lazyRouteComponent(screens.agents, "AgentsScreen"),
});

const tasksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tasks",
  component: lazyRouteComponent(screens.tasks, "TasksScreen"),
});

const peopleRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/people",
  component: lazyRouteComponent(screens.people, "PeopleScreen"),
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/analytics",
  component: lazyRouteComponent(screens.analytics, "AnalyticsScreen"),
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/models",
  component: lazyRouteComponent(screens.models, "ModelsScreen"),
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: lazyRouteComponent(screens.settings, "SettingsScreen"),
});

const routeTree = rootRoute.addChildren([
  homeRoute,
  recordRoute,
  libraryRoute,
  pageRoute,
  meetingRoute,
  notesRoute,
  agendaRoute,
  agentsRoute,
  tasksRoute,
  chatRoute,
  peopleRoute,
  analyticsRoute,
  modelsRoute,
  settingsRoute,
]);

export const router = createRouter({
  routeTree,
  history,
  // Shown while a screen's chunk is in flight, and only when that takes long enough to be worth
  // saying something about. The router's own default is a second of nothing followed by half a
  // second of the pending component, which is right here: locally a chunk arrives in a few
  // milliseconds, so the screen simply appears and this is never rendered.
  defaultPendingComponent: ScreenPending,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
