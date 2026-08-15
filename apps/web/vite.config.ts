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
         * One name, and the rest left to the bundler.
         *
         * There used to be a `vendor` chunk holding every package that was not React, and it was
         * costing what it was meant to save. `manualChunks` is a *forcing* function: naming a
         * module puts it in that chunk no matter who imports it, so a dialog used only by the
         * settings screen, a chart used only by analytics and the animation engine behind a lazy
         * import all landed in a file the entry point preloads. It had to be kept honest with a
         * growing list of exclusions — tiptap, its CRDT, the Tauri bridge — each added after
         * somebody noticed a first load had grown, and each one a package that would have been
         * placed correctly by leaving it alone.
         *
         * Rollup already does this: a module reachable only from a dynamic import goes into that
         * import's chunk, and a module two chunks share is hoisted into one they both fetch. The
         * screens are dynamic imports (see `src/router.tsx`), so what the entry preloads is now
         * what the entry actually uses — 208 kB gzipped rather than 277.
         *
         * React keeps a name because it is the exception the rule is about: everything imports it,
         * so it is hoisted anyway, and pinning it makes it one file that does not change between
         * releases. Its scheduler and `react-dom` go with it — split from it, the renderer would
         * wait for a second file before it could do anything at all.
         */
        manualChunks(id: string) {
          if (!id.includes("node_modules")) return undefined;
          if (/[\\/]node_modules[\\/](react|react-dom|scheduler)[\\/]/.test(id)) return "react";
          return undefined;
        },
      },
    },
  },
  server: { port: 5173, strictPort: true },
});
