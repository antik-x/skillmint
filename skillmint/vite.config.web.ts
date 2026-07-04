import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

// Web-only config for browser click-testing via chrome-devtools-mcp.
// Aliases the Tauri API modules to local mocks that talk to tools/mock_server.py
// (which reads the live SQLite DB). Use with: npx vite --config vite.config.web.ts
export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@tauri-apps/api/core": path.resolve(__dirname, "./src/mock/core.ts"),
      "@tauri-apps/plugin-dialog": path.resolve(__dirname, "./src/mock/dialog.ts"),
      "@tauri-apps/plugin-fs": path.resolve(__dirname, "./src/mock/fs.ts"),
    },
  },
  server: {
    port: 1422,
    strictPort: true,
    host: "127.0.0.1",
  },
});
