import { Component, Suspense, createElement, lazy, useState, type ComponentProps, type ComponentType, type ReactNode } from "react"
import { LoadingSurface, type SkeletonLayout } from "@/components/ui/LoadingSurface"
import { JoluButton } from "@/components/ui/JoluButton"
import { useContentMotion } from "@/lib/motion"

class PageLoadError extends Error {}

class PageErrorBoundary extends Component<{ children: ReactNode; label: string; onRetry: () => void }, { failed: boolean; loadFailed: boolean }> {
  state = { failed: false, loadFailed: false }

  static getDerivedStateFromError(error: unknown) { return { failed: true, loadFailed: error instanceof PageLoadError } }

  render() {
    return this.state.failed
      ? <div role="alert" className="flex items-center justify-between gap-3 rounded-[10px] bg-jolu-danger-soft px-4 py-2.5 text-xs text-jolu-danger">
          <span>{this.props.label}暂时无法显示。{this.state.loadFailed ? "页面资源加载失败，可以重新加载应用，或切换到其他页面。" : "请重试。"}</span>
          <JoluButton size="sm" variant="danger-soft" onClick={this.state.loadFailed ? () => window.location.reload() : this.props.onRetry}>
            {this.state.loadFailed ? "重新加载应用" : "重试"}
          </JoluButton>
        </div>
      : this.props.children
  }
}

function ReadyPage({ children, fillHeight }: { children: ReactNode; fillHeight: boolean }) {
  const motion = useContentMotion("ready", 220, 0, true)
  return <div ref={motion} className={fillHeight ? "h-full min-h-0 min-w-0" : "min-w-0"}>{children}</div>
}

/** 在模块顶层创建；普通重渲染复用同一 lazy 实例，只有明确重试才丢弃缓存的拒绝结果。 */
export function createLazyPage<T extends ComponentType<any>>(load: () => Promise<{ default: T }>, label: string, layout: SkeletonLayout, fillHeight = false) {
  // 浏览器会缓存失败的模块请求；重新创建 lazy 无法清除它，资源错误必须由用户主动重载应用。
  const loadPage = () => load().catch((cause: unknown) => { throw new PageLoadError("页面资源加载失败", { cause }) })
  let current = lazy(loadPage)
  return function LazyPage(props: ComponentProps<T>) {
    const [Page, setPage] = useState(() => current)
    const [attempt, setAttempt] = useState(0)
    const retry = () => {
      current = lazy(loadPage)
      setPage(() => current)
      setAttempt((value) => value + 1)
    }
    return (
      <PageErrorBoundary key={attempt} label={label} onRetry={retry}>
        <Suspense fallback={<LoadingSurface className={fillHeight ? "h-full" : undefined} layout={layout} label={`正在加载${label}`} />}>
          <ReadyPage fillHeight={fillHeight}>{createElement(Page, props)}</ReadyPage>
        </Suspense>
      </PageErrorBoundary>
    )
  }
}
