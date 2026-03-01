import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    proxy: {
      "/api/v1/events/stream": {
        target: "http://localhost:8080",
        // Prevent proxy from buffering the SSE response
        configure: (proxy) => {
          proxy.on("proxyRes", (proxyRes) => {
            proxyRes.headers["X-Accel-Buffering"] = "no";
            proxyRes.headers["Cache-Control"] = "no-cache, no-transform";
          });
        },
      },
      "/api": "http://localhost:8080",
      "/.well-known": "http://localhost:8080",
    },
  },
});
