import {
  Outlet,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { claimHandshake } from "./lib/session";
import { RootLayout } from "./components/shell/RootLayout";
import { AgendaScreen } from "./screens/AgendaScreen";
import { HomeScreen } from "./screens/HomeScreen";
import { AnalyticsScreen } from "./screens/AnalyticsScreen";
import { AgentsScreen } from "./screens/AgentsScreen";
import { ChatScreen } from "./screens/ChatScreen";
import { LibraryScreen } from "./screens/LibraryScreen";
import { PageScreen } from "./screens/PageScreen";
import { NotesScreen } from "./screens/NotesScreen";
import { PeopleScreen } from "./screens/PeopleScreen";
import { RecordScreen } from "./screens/RecordScreen";
import { TasksScreen } from "./screens/TasksScreen";
import { ModelsScreen } from "./screens/ModelsScreen";
import { SettingsScreen } from "./screens/SettingsScreen";

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
  component: RecordScreen,
});

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: LibraryScreen,
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
  component: PageScreen,
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
  component: NotesScreen,
  // `?open=` so a note is a link. Without it the sidebar could list every page and open none of
  // them, which is a table of contents for a book with no page numbers.
  validateSearch: (search: Record<string, unknown>) => ({
    open: typeof search.open === "string" && search.open ? search.open : undefined,
  }),
});

const agendaRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agenda",
  component: AgendaScreen,
});

const chatRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/chat",
  component: ChatScreen,
});

const agentsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/agents",
  component: AgentsScreen,
});

const tasksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/tasks",
  component: TasksScreen,
});

const peopleRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/people",
  component: PeopleScreen,
});

const analyticsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/analytics",
  component: AnalyticsScreen,
});

const modelsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/models",
  component: ModelsScreen,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsScreen,
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

export const router = createRouter({ routeTree, history });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
