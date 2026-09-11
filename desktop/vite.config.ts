import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri 要求固定端口，且忽略 src-tauri 目录的变更（由 tauri dev 自行重编）
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome120", // Windows 端 WebView2 基线
    sourcemap: false,
  },
});
