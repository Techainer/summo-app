/**
 * The animation engine, in a file of its own so it can be fetched separately.
 *
 * A one-line re-export looks pointless and is not. `LazyMotion` takes a function returning the
 * feature bundle, and writing that as `() => import("motion/react").then((m) => m.domAnimation)`
 * asks the bundler for the whole module — including the eager `motion` components — which is the
 * thing being avoided. A module whose only export is the bundle has nothing else to drag along.
 *
 * `domAnimation` and not `domMax`: the difference is layout animations and drag, and Summo uses
 * neither. Dragging a page onto a folder in the sidebar is HTML5 drag-and-drop, which the browser
 * does. Buying `domMax` for that would be paying for a second implementation of it.
 */
export { domAnimation as default } from "motion/react";
