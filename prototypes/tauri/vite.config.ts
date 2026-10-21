import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Tauri serves the built files itself; the dev server is only for `tauri dev`.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [solid()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: ["es2021", "chrome100"],
    minify: "esbuild",
    sourcemap: false,
    assetsInlineLimit: 0,
  },
});
