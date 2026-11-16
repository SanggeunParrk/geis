import { defineConfig } from "vite";

// Tauri drives this dev server, so the port is fixed and failures must be loud rather
// than silently moving to another port the webview will not be pointed at.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "safari15",
    sourcemap: true,
  },
});
