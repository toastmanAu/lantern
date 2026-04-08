// apps/desktop/vite.config.ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { TanStackRouterVite } from "@tanstack/router-vite-plugin";

// https://vite.dev/config/
// Tauri expects a fixed dev port and serves /src-tauri assets relative to root
export default defineConfig({
  plugins: [TanStackRouterVite({ target: "react", autoCodeSplitting: true }), react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: !process.env["TAURI_DEBUG"] && "esbuild",
    sourcemap: !!process.env["TAURI_DEBUG"],
    outDir: "dist",
  },
});
