import type { ReactNode } from "react"
import { cn } from "@/lib/utils"
import { useContentMotion } from "@/lib/motion"

export type SkeletonLayout = "stats" | "providers" | "accounts" | "calendar" | "island" | "tasks" | "table" | "cards" | "settings" | "sessions" | "messages"

function Bar({ className }: { className?: string }) {
  return <span aria-hidden className={cn("skeleton-bar block h-3 rounded-md", className)} />
}

/** 首帧显示布局占位；等待期间持续柔光，不人为延迟已就绪的数据。 */
export function LoadingSurface({ layout = "table", label = "正在加载", className }: {
  layout?: SkeletonLayout; label?: string; className?: string
}) {
  const rows = <div className="flex flex-col gap-5">{Array.from({ length: 5 }, (_, i) => <div key={i} className="flex items-center gap-4"><Bar className="size-8 shrink-0" /><div className="flex flex-1 flex-col gap-2"><Bar className={i % 2 ? "w-2/3" : "w-1/2"} /><Bar className="h-2 w-1/3" /></div><Bar className="w-16" /></div>)}</div>
  const card = <div className="flex h-[100px] flex-col gap-2 rounded-xl border border-jolu-border bg-jolu-bg p-3.5"><Bar className="w-1/3" /><Bar className="h-7 w-2/3" /><Bar className="h-2 w-1/2" /></div>
  return <div role="status" aria-label={label} aria-busy="true" className={cn("loading-surface min-w-0 w-full", className)}>
    <span className="sr-only">{label}</span>
    <div aria-hidden className="skeleton-content flex flex-col gap-4">
      {layout === "providers" ? <><Bar className="h-7 w-44" /><div className="grid grid-cols-4 gap-4">{Array.from({ length: 10 }, (_, i) => <div key={i} className="flex h-[160px] flex-col gap-4 rounded-xl border border-jolu-border p-4"><Bar className="size-6" /><Bar className="w-2/3" /><Bar className="h-8 w-full" /></div>)}</div></>
      : layout === "accounts" ? <><Bar className="h-7 w-44" /><Bar className="h-9 w-2/3" /><div className="grid grid-cols-4 gap-4">{[0,1,2,3].map(i => <div key={i} className="flex min-h-[470px] flex-col gap-5 rounded-xl border border-jolu-border p-4"><Bar className="size-9" /><Bar className="w-3/4" /><Bar className="h-7" /><Bar className="h-24" /><Bar className="w-2/3" /><Bar className="w-1/2" /><Bar className="mt-auto h-8" /></div>)}</div></>
      : layout === "calendar" ? <div className="flex h-[175px] flex-col gap-2"><Bar className="h-4 w-3/4" /><div className="skeleton-calendar grid flex-1 grid-cols-[repeat(53,minmax(0,1fr))] gap-[3px]">{Array.from({ length: 53 }, (_, i) => <div key={i} className="flex flex-col gap-[3px]">{Array.from({ length: 7 }, (_, j) => <Bar key={j} className="h-[19px] rounded-[3px]" />)}</div>)}</div></div>
      : layout === "island" ? <><Bar className="h-[60px]" /><div className="grid grid-cols-2 gap-5"><Bar className="h-[210px]" /><Bar className="h-[210px]" /></div><Bar className="h-[150px]" /><Bar className="h-[60px]" /><Bar className="h-[60px]" /></>
      : layout === "tasks" ? <><div className="grid grid-cols-4 gap-3">{[0,1,2,3].map(i => <div key={i}>{card}</div>)}</div><div className="rounded-xl border border-jolu-border p-4">{rows}</div></>
      : layout === "stats" ? <><div className="grid grid-cols-4 gap-3">{[0, 1, 2, 3].map(i => <div key={i}>{card}</div>)}</div><div className="grid grid-cols-[2fr_1fr] gap-4"><Bar className="h-[250px]" /><Bar className="h-[250px]" /></div><div className="rounded-xl border border-jolu-border p-4">{rows}</div></>
      : layout === "cards" ? <div className="grid grid-cols-2 gap-4">{[0, 1, 2, 3].map(i => <div key={i}>{card}</div>)}</div>
      : layout === "sessions" ? <div className="grid min-h-[600px] grid-cols-[1fr_2fr] gap-4"><div className="rounded-xl border border-jolu-border p-4">{rows}</div><div className="flex flex-col gap-6 rounded-xl border border-jolu-border p-5"><Bar className="w-1/3" /><Bar className="h-20 w-full" /><Bar className="h-32 w-4/5" /></div></div>
      : layout === "messages" ? <div className="flex min-h-52 flex-col gap-5"><Bar className="h-16 w-4/5 self-end" /><Bar className="h-24 w-full" /><Bar className="h-12 w-3/4" /></div>
      : <div className={cn("rounded-xl border border-jolu-border bg-jolu-bg p-5", layout === "settings" && "min-h-[360px]")}>{rows}</div>}
    </div>
  </div>
}

/** 不重挂载内容；刷新仍显示原数据，只在页面标识或首次就绪变化时过渡。 */
export function ContentTransition({ identity, children, className }: { identity: string; children: ReactNode; className?: string }) {
  const ref = useContentMotion(identity)
  return <div ref={ref} className={cn("min-w-0", className)}>{children}</div>
}
