import { useState } from "react"
import { ChevronLeft, ChevronRight } from "lucide-react"
import { cn } from "@/lib/utils"
import { parsePageInput } from "@/lib/paginationState"
import { pageWindow } from "@/lib/usageStats"

/**
 * 分页栏（画板 34「调用明细」原样式；统一网关路由流水复用）：
 * 统计、上一页、页码窗口与省略号、下一页、页码输入与跳转。
 * 首页禁用上一页、末页禁用下一页；省略号不可点；非法页码不翻页并提示有效范围；
 * `busy` 时阻止重复提交。没有数据（pageCount 为 0）时不显示可操作的页码。
 */
export function Pagination({ page, pageCount, pageSize, busy = false, summary, onPage }: {
  page: number
  pageCount: number
  pageSize: number
  busy?: boolean
  /** 左侧统计文案；默认「每页 N 条 · 共 M 页」 */
  summary?: string
  onPage: (page: number) => void
}) {
  const [draft, setDraft] = useState("")
  const [error, setError] = useState<string | null>(null)
  const box = "grid size-8 place-items-center rounded-lg border text-xs outline-none focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45"
  const go = (target: number) => {
    if (busy || target === page || target < 1 || target > pageCount) return
    onPage(target)
  }
  const jump = () => {
    if (busy) return
    const parsed = parsePageInput(draft, pageCount)
    if (parsed.error !== null) return setError(parsed.error)
    setError(null)
    setDraft("")
    go(parsed.page)
  }
  return (
    <div className="flex h-[60px] items-center gap-2 px-4" aria-busy={busy}>
      {/* 页码输入有误时，这里临时换成错误提示 */}
      <span role={error ? "alert" : undefined} className={cn("text-[11px]", error ? "text-jolu-danger" : "text-jolu-muted")}>
        {error ?? summary ?? `每页 ${pageSize} 条 · 共 ${pageCount} 页`}
      </span>
      <span className="flex-1" />
      {pageCount > 0 && (
        <>
          <button type="button" aria-label="上一页" disabled={busy || page <= 1} onClick={() => go(page - 1)} className={cn(box, "border-jolu-border bg-jolu-bg text-jolu-muted hover:bg-jolu-stage")}><ChevronLeft aria-hidden className="size-3.5" /></button>
          {pageWindow(page, pageCount).map((p, index) => p === "…"
            ? <span key={`gap-${index}`} className="grid h-8 w-6 place-items-center text-xs text-jolu-muted">…</span>
            : (
              <button key={p} type="button" aria-current={p === page ? "page" : undefined} disabled={busy && p !== page} onClick={() => go(p)}
                className={cn(box, p === page ? "border-jolu-accent bg-jolu-accent font-semibold text-[#071B10] disabled:opacity-100" : "border-jolu-border bg-jolu-bg font-medium text-jolu-text hover:bg-jolu-stage")}>
                {p}
              </button>
            ))}
          <button type="button" aria-label="下一页" disabled={busy || page >= pageCount} onClick={() => go(page + 1)} className={cn(box, "border-jolu-border bg-jolu-bg text-jolu-muted hover:bg-jolu-stage")}><ChevronRight aria-hidden className="size-3.5" /></button>
          <input
            value={draft}
            inputMode="numeric"
            placeholder="页码"
            aria-label="跳转页码"
            aria-invalid={!!error}
            onChange={(event) => { setDraft(event.target.value); setError(null) }}
            onKeyDown={(event) => event.key === "Enter" && jump()}
            className={cn("h-8 w-[76px] rounded-lg border bg-jolu-bg px-3 text-xs text-jolu-text outline-none placeholder:text-jolu-muted focus-visible:border-jolu-accent", error ? "border-jolu-danger" : "border-jolu-border")}
          />
          <button type="button" disabled={busy} onClick={jump} className="jolu-control h-8 w-[54px] rounded-lg border border-jolu-border bg-jolu-bg text-xs font-medium text-jolu-text outline-none hover:bg-jolu-stage focus-visible:outline-2 focus-visible:outline-jolu-accent disabled:cursor-not-allowed disabled:opacity-45">跳转</button>
        </>
      )}
    </div>
  )
}
