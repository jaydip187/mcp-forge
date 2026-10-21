/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import path from "node:path";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// `npm run dev` proxies API/MCP calls to a locally running `mcp-forge serve`.
const backend = process.env.MCP_FORGE_URL ?? "http://127.0.0.1:7410";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  build: { outDir: "dist", emptyOutDir: true, sourcemap: false },
  server: {
    proxy: {
      "/api": { target: backend, changeOrigin: false },
      "/mcp": { target: backend, changeOrigin: false },
    },
  },
  test: { environment: "node" },
});
