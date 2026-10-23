import { useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { api, isTauri, listenEvent, type UpdateCheckDto, type UpdateProgressDto } from "@/lib/api"
import {
  UPDATES_ENABLED,
  isUpdateRelevant,
  readDismissedVersion,
  writeDismissedVersion,
  type UpdateInfo,
  type UpdatePhase,
  type UpdateProgress,
} from "@/lib/updateState"
import { useSettings } from "@/hooks/useSettings"
import { UpdateContext, type UpdateContextValue } from "./UpdateContextBase"

/**
 * 更新状态（§更新，画布 17）。
 *
 * 两个更新入口共用状态：先下载并校验，再由安装界面的按钮安装。
 * 浏览器预览（非 Tauri）走内置模拟流程，便于对照画布验收交互。
 *
 * Context 对象在 ./UpdateContextBase（独立文件，防 HMR 实例分裂），
 * 这里 re-export 保持既有 import 路径不变。
 */

export { UpdateContext }
export type { UpdateContextValue }


const MOCK_INFO: UpdateInfo = {
  currentVersion: "0.1.0",
  availableVersion: "0.2.0",
  notes: "灵动岛新增会话耗时角标；修复代理转发偶发中断；连接管理支持批量导入。",
  pubDate: "2026-09-18",
}

export function UpdateProvider({ children }: { children: ReactNode }) {
  const cfg = useSettings()
  const [phase, setPhase] = useState<UpdatePhase>("idle")
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [progress, setProgress] = useState<UpdateProgress>({ downloaded: 0, total: null })
  const [dismissed, setDismissed] = useState<string | null>(() => readDismissedVersion())
  const [error, setError] = useState<string | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  const cancelRequested = useRef(false)
  const busy = useRef(false)
  const timers = useRef<number[]>([])

  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), [])

  const applyCheck = useCallback((dto: UpdateCheckDto) => {
    if (!dto.available) {
      setInfo(null)
      return false
    }
    setInfo({
      currentVersion: dto.current_version,
      availableVersion: dto.available_version,
      notes: dto.notes,
      pubDate: dto.pub_date,
    })
    return true
  }, [])

  const checkNow = useCallback(async () => {
    if (!UPDATES_ENABLED || busy.current) return false
    if (phase === "downloaded") return true
    busy.current = true
    setPhase("checking")
    setError(null)
    try {
      if (!isTauri) {
        // 预览：按画布示例固定给出 v0.2.0
        setInfo(MOCK_INFO)
        return true
      }
      return applyCheck(await api.checkAppUpdate())
    } catch (reason) {
      setError(`检查更新失败：${String(reason)}`)
      throw reason
    } finally {
      busy.current = false
      setPhase((current) => (current === "checking" ? "idle" : current))
    }
  }, [applyCheck, phase])

  const startDownload = useCallback(() => {
    if (busy.current) return
    setDialogOpen(true)
    if (phase === "downloaded") return
    cancelRequested.current = false
    if (!isTauri) {
      // 预览：模拟下载，完成后等待用户点击安装。
      busy.current = true
      setPhase("downloading")
      setError(null)
      setProgress({ downloaded: 0, total: 18.2 * 1024 * 1024 })
      const step = 1024 * 1024
      const tick = (downloaded: number) => {
        if (cancelRequested.current) return
        setProgress({ downloaded: Math.min(downloaded, 18.2 * 1024 * 1024), total: 18.2 * 1024 * 1024 })
        if (downloaded < 18.2 * 1024 * 1024) {
          timers.current.push(window.setTimeout(() => tick(downloaded + step * 2), 120))
        } else {
          setPhase("downloaded")
          busy.current = false
        }
      }
      timers.current.push(window.setTimeout(() => tick(step * 2), 200))
      return
    }
    busy.current = true
    setPhase("downloading")
    setProgress({ downloaded: 0, total: null })
    setError(null)
    let off: (() => void) | undefined
    const unlisten = listenEvent<UpdateProgressDto>("update-download-progress", (p) => {
      setProgress({ downloaded: p.downloaded, total: p.total })
    })
    void unlisten
      .then((unsubscribe) => {
        off = unsubscribe
        if (cancelRequested.current) throw new Error("更新下载已取消")
        return api.downloadAppUpdate()
      })
      .then((dto) => {
        applyCheck(dto)
        setPhase("downloaded")
      })
      .catch((reason) => {
        if (cancelRequested.current || String(reason).includes("更新下载已取消")) {
          setError(null)
          setPhase("idle")
          setDialogOpen(false)
        } else {
          setError(`下载更新失败：${String(reason)}`)
          setPhase("downloadError")
        }
      })
      .finally(() => {
        busy.current = false
        off?.()
      })
  }, [applyCheck, phase])

  const closeDialog = useCallback(() => {
    if (busy.current) return
    setDialogOpen(false)
    if (phase === "downloadError" || phase === "ready") setPhase("idle")
  }, [phase])

  const cancelDownload = useCallback(() => {
    if (phase !== "downloading") return
    cancelRequested.current = true
    if (!isTauri) {
      timers.current.forEach((timer) => window.clearTimeout(timer))
      timers.current = []
      busy.current = false
      setPhase("idle")
      setDialogOpen(false)
      return
    }
    void api.cancelAppUpdateDownload().catch((reason) => {
      cancelRequested.current = false
      setError(`取消下载失败：${String(reason)}`)
    })
  }, [phase])

  const installDownloaded = useCallback(() => {
    if (busy.current) return
    busy.current = true
    setError(null)
    setPhase("installing")
    if (!isTauri) {
      timers.current.push(window.setTimeout(() => {
        setPhase("ready")
        busy.current = false
      }, 900))
      return
    }
    void api.installDownloadedUpdate()
      .then(() => setPhase("ready"))
      .catch((reason) => {
        setError(`安装更新失败：${String(reason)}`)
        setPhase("downloaded")
      })
      .finally(() => { busy.current = false })
  }, [])

  const dismiss = useCallback(() => {
    if (!info) return
    writeDismissedVersion(info.availableVersion)
    setDismissed(info.availableVersion)
  }, [info])

  const checkRef = useRef(checkNow)
  checkRef.current = checkNow

  // 启动 1 秒后静默检查；失败不打扰（画布标注：静默重试下次启动）
  useEffect(() => {
    if (!UPDATES_ENABLED || !cfg.ready || !cfg.settings.auto_check_updates) return
    const timer = window.setTimeout(() => {
      void checkRef.current().catch(() => {})
    }, 1000)
    return () => window.clearTimeout(timer)
  }, [cfg.ready, cfg.settings.auto_check_updates])

  const value = useMemo<UpdateContextValue>(
    () => ({
      dialogOpen,
      closeDialog,
      cancelDownload,
      phase,
      info,
      progress,
      visible:
        (phase === "idle" || phase === "available") && isUpdateRelevant(info?.availableVersion ?? null, dismissed),
      error,
      checkNow,
      startDownload,
      installDownloaded,
      dismiss,
    }),
    [dialogOpen, closeDialog, cancelDownload, phase, info, progress, dismissed, error, checkNow, startDownload, installDownloaded, dismiss],
  )

  return <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>
}

export function useUpdate(): UpdateContextValue {
  const value = useContext(UpdateContext)
  if (!value) throw new Error("useUpdate 必须在 UpdateProvider 内使用")
  return value
}
