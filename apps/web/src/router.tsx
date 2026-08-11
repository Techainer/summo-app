import {
  Outlet,
  createHashHistory,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";

import { RootLayout } from "./components/shell/RootLayout";
import { AgendaScreen } from "./screens/AgendaScreen";
import { AnalyticsScreen } from "./screens/AnalyticsScreen";
import { ChatScreen } from "./screens/ChatScreen";
import { LibraryScreen } from "./screens/LibraryScreen";
import { MeetingScreen } from "./screens/MeetingScreen";
import { NotesScreen } from "./screens/NotesScreen";
import { PeopleScreen } from "./screens/PeopleScreen";
import { RecordScreen } from "./screens/RecordScreen";
import { TasksScreen } from "./screens/TasksScreen";
import { SettingsScreen } from "./screens/SettingsScreen";

/**
 * Hash history, not browser history.
 *
 * The desktop shell serves the built assets from a custom scheme (`tauri://`) with a relative base,
 * where pushing real paths asks the webview for a document that does not exist. A hash keeps every
 * route inside one document while still giving real URLs, back/forward, and deep links into a
 * meeting.
 */
const history = createHashHistory();

export interface LibrarySearch {
  folder?: string;
  tag?: string;
  q?: string;
}

const rootRoute = createRootRoute({
  component: () => (
    <RootLayout>
      <Outlet />
    </RootLayout>
  ),
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
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
    return { folder: text(search.folder), tag: text(search.tag), q: text(search.q) };
  },
});

const meetingRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/meetings/$meetingId",
  component: MeetingScreen,
});

const notesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/notes",
  component: NotesScreen,
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

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsScreen,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  libraryRoute,
  meetingRoute,
  notesRoute,
  agendaRoute,
  tasksRoute,
  chatRoute,
  peopleRoute,
  analyticsRoute,
  settingsRoute,
]);

export const router = createRouter({ routeTree, history });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
