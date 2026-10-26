import path from "node:path"
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { "@": path.resolve(__dirname, "./src") },
  },
  // 5173 被占用时顺延到下一个端口；pnpm tauri dev 由 scripts/dev/tauri.mjs 选好端口后以 --strictPort 启动。构建产物输出到 dist 供 Tauri 打包
  server: { port: 5173 },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: { input: { main: path.resolve(__dirname, "index.html"), startup: path.resolve(__dirname, "startup.html") } },
  },
})
