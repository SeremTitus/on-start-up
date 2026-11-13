import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed dev server port and doesn't want vite to clear the
// terminal output of the Rust build.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Don't watch the Cargo workspace: watching in-flight build artifacts
      // crashes the watcher (EBUSY) and would trigger useless reloads.
      ignored: ["**/target/**", "**/node_modules/**", "**/dist/**", "**/icons/**"],
    },
  },
  build: {
    target: "es2021",
  },
});
