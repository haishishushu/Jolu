import { useCallback, useEffect, useRef, useState } from "react"
import { api, type LoginDetectionDto } from "@/lib/api"

/** 授权 ID 不是凭证；换平台、换区域、关闭时取消，迟到结果不能覆盖当前草稿。 */
export function useBrowserAuthorization(platform: string, edition: string, enabled: boolean) {
  const revision = useRef(0)
  const id = useRef<string | null>(null)
  const busy = useRef(false)
  const [pending, setPending] = useState(false)
  const [detection, setDetection] = useState<LoginDetectionDto | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [message, setMessage] = useState<string | null>(null)
  const supported = ["workbuddy", "qoder", "trae", "claude", "codex", "gemini", "grok", "opencode", "zcode"].includes(platform)

  const release = useCallback(() => {
    ++revision.current
    busy.current = false
    const current = id.current
    id.current = null
    if (current) void api.cancelBrowserAuthorization(current).catch(() => {})
  }, [])
  const cancel = useCallback(() => {
    release()
    setPending(false)
    setDetection(null)
    setError(null)
    setMessage(null)
  }, [release])

  useEffect(() => {
    cancel()
    return release
  }, [platform, edition, enabled, cancel, release])

  const start = async () => {
    if (!enabled || !supported || busy.current) return
    release()
    const request = revision.current
    busy.current = true
    setPending(true)
    setDetection(null)
    setError(null)
    setMessage(null)
    try {
      const current = await api.startBrowserAuthorization(platform, edition)
      if (request !== revision.current) { void api.cancelBrowserAuthorization(current).catch(() => {}); return }
      id.current = current
      while (request === revision.current) {
        const status = await api.browserAuthorizationStatus(current)
        if (request !== revision.current) return
        if (status.state === "ready" && status.detection?.found) { setDetection(status.detection); return }
        if (status.state !== "waiting") throw new Error(status.message ?? "授权未完成，请重新发起")
        setMessage(status.message)
        await new Promise(resolve => window.setTimeout(resolve, 2_000))
      }
    } catch (reason) {
      if (request === revision.current) setError(String(reason))
    } finally {
      if (request === revision.current) { busy.current = false; setPending(false) }
    }
  }

  const save = async (name: string) => {
    if (!id.current || !detection?.found) throw new Error("浏览器授权尚未验证通过")
    await api.saveBrowserConnection(id.current, name)
  }
  return { pending, detection, error, message, supported, start, cancel, save }
}
