import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  // Assets are served from the gateway's own namespace, not from wherever the
  // page is mounted: the console answers paths like /account/login, and a
  // relative URL there would resolve to /account/assets/... and 404. This
  // prefix also cannot collide with anything the PDS serves.
  base: "/_gateway/console/",
  plugins: [react(), tailwindcss()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
  test: {
    globals: true,
    environment: "happy-dom",
    setupFiles: ["src/test/setup.ts"],
    css: false,
    include: ["src/**/*.test.{ts,tsx}"],
  },
  server: {
    proxy: {
      "/xrpc": { target: "http://127.0.0.1:2583", changeOrigin: true },
    },
  },
});
