/// <reference types="vitest/config" />
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Porta fixa: o Tauri aponta o devUrl para ela.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    // Desenvolvimento da interface Web: encaminha para o OneHub Central em loopback (sem CORS).
    proxy: { "/central-api": { target: "http://127.0.0.1:8787", rewrite: (path) => path.replace(/^\/central-api/, "") } },
  },
  build: {
    target: "safari15",
    sourcemap: false,
    rollupOptions: { input: { main: "index.html", central: "central.html" } },
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
