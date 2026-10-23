import { useEffect, useState } from "react"

/** 系统是否开启了「减少动画」；随系统设置变化实时更新 */
export function useSystemReducedMotion(): boolean {
  const [reduce, setReduce] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches)
  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)")
    const changed = () => setReduce(query.matches)
    query.addEventListener("change", changed)
    changed()
    return () => query.removeEventListener("change", changed)
  }, [])
  return reduce
}
