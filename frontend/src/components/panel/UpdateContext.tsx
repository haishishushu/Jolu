import { useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import { api, isTauri, listenEvent, type UpdateCheckDto, type UpdateProgressDto } from "@/lib/api"
import {
  AUTO_CHECK_INTERVAL_MS,
  UPDATES_ENABLED,
  isUpdateRelevant,
  needsDownload,
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
 * 启动后检查一次、之后每 30 分钟检查一次；发现新版本自动在后台下载并校验，
 * 页头按钮变绿显示版本号，点击即进入安装界面（NSIS 被动模式，装完自动重启）。
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

const MOCK_TOTAL = 18.2 * 1024 * 1024

export function UpdateProvider({ children }: { children: ReactNode }) {
  const cfg = useSettings()
  const [phase, setPhaseState] = useState<UpdatePhase>("idle")
  const [info, setInfo] = useState<UpdateInfo | null>(null)
  const [progress, setProgress] = useState<UpdateProgress>({ downloaded: 0, total: null })
  const [dismissed, setDismissed] = useState<string | null>(() => readDismissedVersion())
  const [error, setError] = useState<string | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)
  const cancelRequested = useRef(false)
  /** 下载或安装进行中（两者在后端互斥） */
  const busy = useRef(false)
  const checking = useRef(false)
  /** 定时检查在异步回调里读取阶段，用 ref 避免闭包拿到旧值 */
  const phaseRef = useRef<UpdatePhase>("idle")
  /** 已下载并校验的安装包版本；有更高版本时据此决定重新下载 */
  const preparedVersion = useRef<string | null>(null)
  const timers = useRef<number[]>([])

  useEffect(() => () => timers.current.forEach((t) => window.clearTimeout(t)), [])

  const setPhase = useCallback((next: UpdatePhase) => {
    phaseRef.current = next
    setPhaseState(next)
  }, [])

  const applyCheck = useCallback((dto: UpdateCheckDto) => {
    if (!dto.available) {
      // 已下载的安装包仍可安装，不因一次检查结果把绿灯熄掉
      if (phaseRef.current !== "downloaded") setInfo(null)
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

  /** 下载并校验安装包；不打开对话框，进度在页头按钮与对话框（若已打开）中显示 */
  const download = useCallback(() => {
    if (busy.current) return
    busy.current = true
    cancelRequested.current = false
    preparedVersion.current = null
    setPhase("downloading")
    setError(null)
    if (!isTauri) {
      // 预览：模拟下载，完成后等待用户点击安装。
      setProgress({ downloaded: 0, total: MOCK_TOTAL })
      const step = 2 * 1024 * 1024
      const tick = (downloaded: number) => {
        if (cancelRequested.current) return
        setProgress({ downloaded: Math.min(downloaded, MOCK_TOTAL), total: MOCK_TOTAL })
        if (downloaded < MOCK_TOTAL) {
          timers.current.push(window.setTimeout(() => tick(downloaded + step), 120))
        } else {
          preparedVersion.current = MOCK_INFO.availableVersion
          setPhase("downloaded")
          busy.current = false
        }
      }
      timers.current.push(window.setTimeout(() => tick(step), 200))
      return
    }
    setProgress({ downloaded: 0, total: null })
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
        preparedVersion.current = dto.available_version
        setPhase("downloaded")
      })
      .catch((reason) => {
        if (cancelRequested.current || String(reason).includes("更新下载已取消")) {
          setError(null)
          setPhase("idle")
          setDialogOpen(false)
        } else {
          // 后台下载失败不打扰：按钮提示可重试，下次定时检查也会自动重试
          setError(`下载更新失败：${String(reason)}`)
          setPhase("downloadError")
        }
      })
      .finally(() => {
        busy.current = false
        off?.()
      })
  }, [applyCheck, setPhase])

  const checkNow = useCallback(async () => {
    if (!UPDATES_ENABLED) return false
    // 下载或安装进行中：已经知道有新版本，不重复请求
    if (busy.current || checking.current) return phaseRef.current !== "idle" && phaseRef.current !== "checking"
    checking.current = true
    if (phaseRef.current === "idle") {
      setPhase("checking")
      setError(null)
    }
    let dto: UpdateCheckDto
    try {
      dto = isTauri
        ? await api.checkAppUpdate()
        : {
          // 预览：按画布示例固定给出 v0.2.0
          available: true,
          current_version: MOCK_INFO.currentVersion,
          available_version: MOCK_INFO.availableVersion,
          notes: MOCK_INFO.notes ?? null,
          pub_date: MOCK_INFO.pubDate ?? null,
        }
    } catch (reason) {
      if (phaseRef.current === "checking") setError(`检查更新失败：${String(reason)}`)
      throw reason
    } finally {
      checking.current = false
      if (phaseRef.current === "checking") setPhase("idle")
    }
    const available = applyCheck(dto)
    if (available && needsDownload(phaseRef.current, preparedVersion.current, dto.available_version)) download()
    return available || phaseRef.current === "downloaded"
  }, [applyCheck, download, setPhase])

  /** 打开进度对话框；未下载或下载失败时同时开始下载 */
  const startDownload = useCallback(() => {
    setDialogOpen(true)
    const current = phaseRef.current
    if (current === "downloaded" || current === "downloading" || current === "installing") return
    download()
  }, [download])

  const closeDialog = useCallback(() => {
    if (busy.current) return
    setDialogOpen(false)
    if (phaseRef.current === "downloadError" || phaseRef.current === "ready") setPhase("idle")
  }, [setPhase])

  const cancelDownload = useCallback(() => {
    if (phaseRef.current !== "downloading") return
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
  }, [setPhase])

  /** 用已下载的安装包进入安装界面；失败（如有运行中的自动化任务）时打开对话框说明原因 */
  const installDownloaded = useCallback(() => {
    if (busy.current || phaseRef.current !== "downloaded") return
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
        setDialogOpen(true)
      })
      .finally(() => { busy.current = false })
  }, [setPhase])

  /** 页头与设置页的版本按钮：已下载直接安装，其余情况打开进度对话框 */
  const openUpdate = useCallback(() => {
    if (phaseRef.current === "downloaded") installDownloaded()
    else startDownload()
  }, [installDownloaded, startDownload])

  const dismiss = useCallback(() => {
    if (!info) return
    writeDismissedVersion(info.availableVersion)
    setDismissed(info.availableVersion)
  }, [info])

  const checkRef = useRef(checkNow)
  checkRef.current = checkNow

  // 启动 1 秒后检查一次，之后每 30 分钟检查一次；失败不打扰，等下一轮
  useEffect(() => {
    if (!UPDATES_ENABLED || !cfg.ready || !cfg.settings.auto_check_updates) return
    const run = () => { void checkRef.current().catch(() => {}) }
    const first = window.setTimeout(run, 1000)
    const interval = window.setInterval(run, AUTO_CHECK_INTERVAL_MS)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(interval)
    }
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
      openUpdate,
      dismiss,
    }),
    [dialogOpen, closeDialog, cancelDownload, phase, info, progress, dismissed, error, checkNow, startDownload, installDownloaded, openUpdate, dismiss],
  )

  return <UpdateContext.Provider value={value}>{children}</UpdateContext.Provider>
}

export function useUpdate(): UpdateContextValue {
  const value = useContext(UpdateContext)
  if (!value) throw new Error("useUpdate 必须在 UpdateProvider 内使用")
  return value
}
