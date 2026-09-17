import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  server: {
    port: 1420,
    strictPort: true,
    // 浏览器开发模式把同源 API 转发给本地 Rust 服务.
    proxy: {
      "/api": {
        // @ts-expect-error process is a nodejs global
        target: process.env.VITE_API_PROXY_TARGET || "http://127.0.0.1:3000",
        changeOrigin: true,
      },
    },
  },
}));
