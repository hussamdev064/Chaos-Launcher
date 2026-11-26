import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects the dev server on a fixed port and serves the built `dist/` in production.
// `clearScreen: false` keeps Rust-side errors visible in the same terminal during `tauri dev`.
export default defineConfig({
  plugins: [tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    sourcemap: true,
  },
});
