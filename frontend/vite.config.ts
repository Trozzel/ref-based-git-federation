import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

function apiPort() {
  return process.env.VITE_API_PORT ?? "8081";
}

function apiBaseUrl(command: string) {
  if (process.env.VITE_API_BASE_URL) {
    return process.env.VITE_API_BASE_URL;
  }

  return command === "build" ? `http://127.0.0.1:${apiPort()}` : "";
}

export default defineConfig(({ command }) => ({
  define: {
    __API_BASE_URL__: JSON.stringify(apiBaseUrl(command))
  },
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: `http://127.0.0.1:${apiPort()}`,
        changeOrigin: true
      }
    }
  }
}));
