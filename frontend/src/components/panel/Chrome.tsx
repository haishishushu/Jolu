import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { api, isTauri } from "@/lib/api"
import { cn } from "@/lib/utils"
import { AppIcon } from "@/components/brand/AppIcon"
import { Wordmark } from "@/components/brand/Wordmark"

/** 主窗口默认 1280 × 900（Jolu.pen 画板 01）；浏览器预览按这个尺寸绘制 */
export const PANEL_WIDTH = 1280
export const PANEL_HEIGHT = 900

/** 窗口图标统一为 10px 大小、1px 线宽。 */
function CaptionIcon({ action, maximized }: { action: "minimize" | "maximize" | "close"; maximized: boolean }) {
  return (
    <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1">
      {action === "minimize" ? (
        <path d="M0 5.5h10" />
      ) : action === "close" ? (
        <path d="m.5.5 9 9m0-9-9 9" />
      ) : maximized ? (
        <>
          <path d="M2.5 2.5v-2h7v7h-2" />
          <rect x=".5" y="2.5" width="7" height="7" />
        </>
      ) : (
        <rect x=".5" y=".5" width="9" height="9" />
      )}
    </svg>
  )
}

export function TitleBar({ title = "Jolu" }: { title?: string }) {
  const [maximized, setMaximized] = useState(false)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    if (!isTauri) return
    let disposed = false
    let unlisten: (() => void) | undefined
    const win = getCurrentWindow()
    const sync = async () => {
      try {
        const value = await win.isMaximized()
        if (!disposed) setMaximized(value)
      } catch {
        if (!disposed) setError("无法读取窗口状态")
      }
    }
    void win.onResized(() => void sync()).then((off) => {
      if (disposed) off()
      else unlisten = off
    }).catch(() => {
      if (!disposed) setError("无法监听窗口状态")
    })
    void sync()
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [])

  const act = async (action: "minimize" | "maximize" | "close") => {
    if (!isTauri) return
    setError(null)
    try {
      const win = getCurrentWindow()
      if (action === "minimize") await win.minimize()
      else if (action === "maximize") {
        await win.toggleMaximize()
        setMaximized(await win.isMaximized())
      } else {
        // 关闭只隐藏主面板（后端拦截关闭请求），托盘再次打开时秒开；灵动岛与后台采集继续运行。
        await win.close()
      }
    } catch {
      setError("窗口操作失败，请重试")
    }
  }

  const actions = [
    { action: "minimize" as const, label: "最小化" },
    { action: "maximize" as const, label: maximized ? "还原" : "最大化" },
    { action: "close" as const, label: "关闭主面板" },
  ]

  return (
    <div
      className="flex h-[30px] w-full shrink-0 select-none items-center justify-between bg-jolu-side pl-3"
      onPointerDown={(event) => {
        if (!isTauri || event.button !== 0 || (event.target as HTMLElement).closest("button")) return
        void api.mainDrag().catch(() => setError("无法拖动窗口"))
      }}
      onDoubleClick={(event) => {
        if ((event.target as HTMLElement).closest("button")) return
        void act("maximize")
      }}
    >
      {/* Jolu.pen 画板 104：品牌只出现在左上角，业务工具都在内容页头 */}
      <div className="flex items-center gap-1.5" title={title}>
        <AppIcon size={24} />
        <Wordmark />
      </div>
      <div className="flex h-full shrink-0 items-center">
        {error && <span role="alert" className="mr-2 text-[11px] text-danger">{error}</span>}
        {actions.map(({ action, label }) => (
          <button
            key={action}
            type="button"
            aria-label={label}
            title={isTauri ? label : `${label}（仅桌面端可用）`}
            disabled={!isTauri}
            onClick={() => void act(action)}
            className={cn("window-caption-button", action === "close" && "window-caption-button-close")}
          >
            <CaptionIcon action={action} maximized={maximized} />
          </button>
        ))}
      </div>
    </div>
  )
}

