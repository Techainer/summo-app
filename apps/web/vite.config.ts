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
          // The editor is 128 kB gzipped and is only reached by opening a note, so it is left to
          // the dynamic import that pulls it in. Naming it here would put it back in `vendor`,
          // which every screen loads — including the one the app starts on.
          if (
            /[\\/]node_modules[\\/](@tiptap|prosemirror-|orderedmap|rope-sequence|w3c-keyname|linkifyjs)/.test(
              id,
            )
          )
            return undefined;
          return "vendor";
        },
      },
    },
  },
  server: { port: 5173, strictPort: true },
});
