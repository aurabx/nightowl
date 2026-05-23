import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri runs the frontend at localhost:5173. We keep the port locked so
// tauri.conf.json's devUrl stays in sync. Mobile-host HMR routing is not
// needed yet — this app is macOS-only.
export default defineConfig({
  plugins: [react(), tailwindcss()],

  // Keep vite's output visible alongside tauri's logs.
  clearScreen: false,

  server: {
    port: 5173,
    strictPort: true,
    watch: {
      // Don't trigger a frontend rebuild on Rust changes.
      ignored: ["**/src-tauri/**"],
    },
  },
});
