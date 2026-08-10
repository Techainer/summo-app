import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Tauri serves the built assets from a custom scheme, so paths must be relative.
  base: "./",
  build: { target: "es2022", sourcemap: true },
  server: { port: 5173, strictPort: true },
});
