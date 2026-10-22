import { useEffect, useState } from "react"

/** 系统当前是否为深色；主题设为「跟随系统」时用它推导实际主题 */
export function useSystemDark(): boolean {
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches)
  useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)")
    const changed = () => setSystemDark(query.matches)
    query.addEventListener("change", changed)
    changed()
    return () => query.removeEventListener("change", changed)
  }, [])
  return systemDark
}
