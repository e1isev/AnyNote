/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and must see Rust-side errors.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    // Old enough for the Chromium on Raspbian 11 (anynote-server's browser UI).
    target: ["chrome87", "firefox78", "safari14"],
    chunkSizeWarningLimit: 2500,
  },
  test: {
    environment: "node",
  },
});
