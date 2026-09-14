import { defineConfig } from "vite";
export default defineConfig({
  server: {
    proxy: {
      "/api": { target: "http://127.0.0.1:8787", ws: true },
      "/hls": "http://127.0.0.1:8787",
      "/media": "http://127.0.0.1:8787",
    },
  },
});
