// pnpm tauri 的入口：dev 时从 5173 起找空闲端口，把 Vite 端口和 devUrl 一起改掉；其他子命令原样转发
import { spawn } from "node:child_process"
import { createRequire } from "node:module"
import net from "node:net"
import { pathToFileURL } from "node:url"

const BASE_PORT = 5173
const MAX_PORT = 65535
// Vite 监听 localhost，可能落在 ::1 或 127.0.0.1，两个都要空闲才算可用
const HOSTS = ["127.0.0.1", "::1"]

function canListen(port, host) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", (error) => resolve(error.code === "EADDRNOTAVAIL" || error.code === "EAFNOSUPPORT"))
    server.once("listening", () => server.close(() => resolve(true)))
    server.listen({ port, host, exclusive: true })
  })
}

export async function findFreePort(start = BASE_PORT) {
  for (let port = start; port <= MAX_PORT; port++) {
    const results = await Promise.all(HOSTS.map((host) => canListen(port, host)))
    if (results.every(Boolean)) return port
  }
  throw new Error(`${start}-${MAX_PORT} 之间没有可用端口`)
}

async function main() {
  const args = process.argv.slice(2)
  if (args[0] === "dev") {
    const port = await findFreePort()
    if (port !== BASE_PORT) console.log(`端口 ${BASE_PORT} 已被占用，改用 ${port}`)
    const config = {
      build: {
        devUrl: `http://localhost:${port}`,
        beforeDevCommand: `pnpm --dir frontend dev --port ${port} --strictPort`,
      },
    }
    args.splice(1, 0, "--config", JSON.stringify(config))
  }
  const cli = createRequire(import.meta.url).resolve("@tauri-apps/cli/tauri.js")
  const child = spawn(process.execPath, [cli, ...args], { stdio: "inherit" })
  child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 0)))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) await main()
