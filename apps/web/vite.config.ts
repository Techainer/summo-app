import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Tauri serves the built assets from a custom scheme, so paths must be relative.
  base: "./",
  build: {
    target: "es2022",
    sourcemap: true,
    /*
     * Rollup's own warning, turned off on purpose — and only because something better replaced it.
     *
     * It fires on any chunk over 500 kB *unminified*, which is a number about the build rather than
     * about the user: it counts the editor's chunk, which nothing fetches until a note is opened,
     * the same as the entry chunk, which everybody fetches before anything appears. `pnpm budget`
     * measures the thing that actually matters — gzipped bytes before the first paint — and fails
     * the build when that grows. Two warnings about size, one of them not answerable, is how a
     * build ends up with warnings everyone ignores.
     */
    chunkSizeWarningLimit: 1000,
    rollupOptions: {
      output: {
        /*
         * Three chunks instead of one, and the warning about a 700 KB bundle goes with them.
         *
         * Not for a network — this file is read off the local disk by a shell that already
         * downloaded it. It is for the *browser*: one 700 KB module has to be parsed and compiled
         * before anything renders, while three are compiled in parallel, and the two vendor chunks
         * never change between releases so a re-render after an update reuses their compiled code.
         *
         * Split by rate of change rather than by size. React and the router move once a quarter;
         * the app moves every day.
         */
        manualChunks(id: string) {
          if (!id.includes("node_modules")) return undefined;
          // React and its scheduler are one unit — splitting them means the renderer waits for a
          // second file before it can do anything at all.
          if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id)) return "react";
          // The editor is 170 kB gzipped and is only reached by opening a note, so it is left to
          // the dynamic import that pulls it in. Naming it here would put it back in `vendor`,
          // which every screen loads — including the one the app starts on.
          //
          // `yjs`, `y-protocols` and `lib0` are on this list for a reason worth writing down: the
          // *drag handle* depends on `@tiptap/extension-collaboration`, which depends on a CRDT.
          // Nothing in Summo is collaborative, and 28 kB gzipped of it arrived because a paragraph
          // can now be picked up. Left in the editor's chunk rather than fought: it is fetched when
          // a note is opened and never on the screen the app starts on. The first-load budget is
          // what caught it — the package list here is easy to forget to extend, and a new
          // transitive dependency lands in `vendor` silently by default.
          if (
            /[\\/]node_modules[\\/](@tiptap|@floating-ui|prosemirror-|orderedmap|rope-sequence|w3c-keyname|linkifyjs|yjs|y-protocols|lib0)/.test(
              id,
            )
          )
            return undefined;
          // The bridge to the desktop and mobile shells, for the same reason. Every use of it is
          // behind `inShell()` and behind a dynamic import, so in a browser — which is what the
          // daemon serves, and what the browser suites run — it is never fetched at all. Naming it
          // here put it in `vendor` and cost every browser user 5 kB of an API for an app they are
          // not running.
          if (/[\\/]node_modules[\\/]@tauri-apps[\\/]/.test(id)) return undefined;
          return "vendor";
        },
      },
    },
  },
  server: { port: 5173, strictPort: true },
});
