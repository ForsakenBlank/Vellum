import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// set by `tauri dev` when the app is served to a phone or another machine
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // keep rust compiler output visible in the same terminal
  clearScreen: false,
  server: {
    // tauri.conf.json points at this port, so fail instead of picking another
    port: 1420,
    strictPort: true,
    host: host ?? false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
  },
  test: {
    // e2e/ holds playwright tests, which run in a real browser instead
    include: ["src/**/*.test.{ts,tsx}"],
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
  },
});
