import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { VitePWA } from "vite-plugin-pwa";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(async () => ({
  plugins: [
    react(),
    tailwindcss(),
    VitePWA({
      // We want explicit control over update timing — autoUpdate can swap
      // the bundle mid-stream during a chat completion, which the user
      // sees as the page going blank halfway through a response.
      registerType: "prompt",
      // Disable in dev so the SW doesn't intercept Vite's HMR.
      devOptions: { enabled: false },
      includeAssets: ["icon-128.png", "icon-256.png", "icon-512.png"],
      manifest: {
        name: "LocalMind",
        short_name: "LocalMind",
        description: "Chat with the LLM running on your computer.",
        start_url: "/",
        scope: "/",
        display: "standalone",
        orientation: "portrait",
        background_color: "#0a0a0b",
        theme_color: "#0a0a0b",
        icons: [
          { src: "/icon-128.png", sizes: "128x128", type: "image/png" },
          { src: "/icon-256.png", sizes: "256x256", type: "image/png" },
          { src: "/icon-512.png", sizes: "512x512", type: "image/png", purpose: "any maskable" },
        ],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,ico,png,svg,webmanifest}"],
        // Take control of clients on first install — without these, the
        // initial SW sits in "waiting" until the next navigation, so an
        // iOS home-screen cold-start (which IS the next navigation) hits
        // the network instead of the cached shell, and shows "can't
        // connect to host" if the desktop is offline.
        skipWaiting: true,
        clientsClaim: true,
        navigateFallback: "/index.html",
        navigateFallbackDenylist: [/^\/api\//, /^\/v1\//, /^\/sd-images\//, /^\/health$/],
        runtimeCaching: [
          {
            urlPattern: ({ url }) => url.pathname === "/api/status",
            handler: "NetworkFirst",
            options: {
              cacheName: "lm-status-v1",
              networkTimeoutSeconds: 3,
              expiration: { maxEntries: 4, maxAgeSeconds: 60 * 60 },
            },
          },
        ],
      },
    }),
  ],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || "0.0.0.0",
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    proxy: {
      "/api": { target: "http://127.0.0.1:3939", changeOrigin: true },
      "/v1": { target: "http://127.0.0.1:3939", changeOrigin: true },
      "/health": { target: "http://127.0.0.1:3939", changeOrigin: true },
      "/sd-images": { target: "http://127.0.0.1:3939", changeOrigin: true },
    },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
